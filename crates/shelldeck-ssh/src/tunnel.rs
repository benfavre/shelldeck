use crate::handler::ForwardedTcpIpEvent;
use crate::session::SharedHandle;
use crate::SshError;
use parking_lot::Mutex as ParkingMutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelStatus {
    Active,
    Error,
    Stopped,
}

pub struct TunnelHandle {
    pub id: Uuid,
    pub status: Arc<ParkingMutex<TunnelStatus>>,
    pub bytes_sent: Arc<AtomicU64>,
    pub bytes_received: Arc<AtomicU64>,
    shutdown_tx: mpsc::Sender<()>,
}

impl TunnelHandle {
    /// Create a proxy handle that shares status/byte counters with a real tunnel
    /// but uses a separate shutdown channel (e.g. to signal a background thread).
    ///
    /// This is used when the real `TunnelHandle` lives inside a `TunnelManager`
    /// on a background thread, and we need a handle on the UI side that can
    /// read status and signal shutdown.
    pub fn new_proxy(
        id: Uuid,
        status: Arc<ParkingMutex<TunnelStatus>>,
        bytes_sent: Arc<AtomicU64>,
        bytes_received: Arc<AtomicU64>,
        shutdown_tx: mpsc::Sender<()>,
    ) -> Self {
        Self {
            id,
            status,
            bytes_sent,
            bytes_received,
            shutdown_tx,
        }
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.try_send(());
    }

    pub fn is_active(&self) -> bool {
        *self.status.lock() == TunnelStatus::Active
    }

    pub fn total_bytes(&self) -> (u64, u64) {
        (
            self.bytes_sent.load(Ordering::Relaxed),
            self.bytes_received.load(Ordering::Relaxed),
        )
    }
}

pub struct TunnelManager {
    tunnels: Vec<TunnelHandle>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            tunnels: Vec::new(),
        }
    }

    /// Check if a local port is available for binding.
    pub async fn check_port_available(port: u16) -> bool {
        TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
    }

    /// Start a local port forward (SSH -L equivalent).
    /// Binds to `local_port` and forwards connections to `remote_host:remote_port` through SSH.
    pub async fn start_local_forward(
        &mut self,
        handle: SharedHandle,
        local_port: u16,
        remote_host: String,
        remote_port: u16,
    ) -> crate::Result<Uuid> {
        if !Self::check_port_available(local_port).await {
            return Err(SshError::PortInUse(local_port));
        }

        let id = Uuid::new_v4();
        let status = Arc::new(ParkingMutex::new(TunnelStatus::Active));
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let bytes_received = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let status_clone = status.clone();
        let bytes_sent_clone = bytes_sent.clone();
        let bytes_received_clone = bytes_received.clone();

        tokio::spawn(async move {
            let listener = match TcpListener::bind(format!("127.0.0.1:{}", local_port)).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind local port {}: {}", local_port, e);
                    *status_clone.lock() = TunnelStatus::Error;
                    return;
                }
            };

            tracing::info!(
                "Local forward: 127.0.0.1:{} -> {}:{}",
                local_port,
                remote_host,
                remote_port
            );

            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, addr)) => {
                                tracing::debug!("Accepted tunnel connection from {}", addr);
                                let handle = handle.clone();
                                let rhost = remote_host.clone();
                                let bs = bytes_sent_clone.clone();
                                let br = bytes_received_clone.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = handle_local_forward_connection(
                                        handle, stream, &rhost, remote_port, bs, br,
                                    )
                                    .await
                                    {
                                        tracing::error!("Forward connection error: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("Accept error: {}", e);
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Stopping local forward on port {}", local_port);
                        break;
                    }
                }
            }

            *status_clone.lock() = TunnelStatus::Stopped;
        });

        self.tunnels.push(TunnelHandle {
            id,
            status,
            bytes_sent,
            bytes_received,
            shutdown_tx,
        });

        Ok(id)
    }

    /// Start a remote port forward (SSH -R equivalent).
    /// Requests the remote to listen on `remote_port` and forward to `local_host:local_port`.
    ///
    /// The `forwarded_rx` receiver delivers server-initiated forwarded-tcpip channels
    /// from the SSH handler. Each incoming connection on the remote port triggers
    /// a `ForwardedTcpIpEvent` which this method's background task uses to connect
    /// to the local target and pipe data bidirectionally.
    pub async fn start_remote_forward(
        &mut self,
        handle: SharedHandle,
        remote_port: u16,
        local_host: String,
        local_port: u16,
        mut forwarded_rx: mpsc::UnboundedReceiver<ForwardedTcpIpEvent>,
    ) -> crate::Result<Uuid> {
        // Request remote forwarding from the server (requires &mut self on Handle)
        {
            let mut h = handle.lock().await;
            h.tcpip_forward("0.0.0.0", remote_port as u32)
                .await
                .map_err(|e| SshError::Tunnel(format!("Remote forward request failed: {}", e)))?;
        }

        let id = Uuid::new_v4();
        let status = Arc::new(ParkingMutex::new(TunnelStatus::Active));
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let bytes_received = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let status_clone = status.clone();
        let bytes_sent_clone = bytes_sent.clone();
        let bytes_received_clone = bytes_received.clone();

        tracing::info!(
            "Remote forward: remote:{} -> {}:{}",
            remote_port,
            local_host,
            local_port
        );

        // Spawn a task that listens for forwarded-tcpip events from the SSH handler
        // and connects each one to the local target.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = forwarded_rx.recv() => {
                        match event {
                            Some(fwd) => {
                                tracing::debug!(
                                    "Remote forward: incoming connection from {}:{} on remote port {} -> forwarding to {}:{}",
                                    fwd.originator_address,
                                    fwd.originator_port,
                                    fwd.connected_port,
                                    local_host,
                                    local_port,
                                );

                                let lhost = local_host.clone();
                                let lport = local_port;
                                let bs = bytes_sent_clone.clone();
                                let br = bytes_received_clone.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = handle_remote_forward_connection(
                                        fwd.channel, &lhost, lport, bs, br,
                                    )
                                    .await
                                    {
                                        tracing::error!(
                                            "Remote forward connection error ({}:{} -> {}:{}): {}",
                                            fwd.originator_address,
                                            fwd.originator_port,
                                            lhost,
                                            lport,
                                            e,
                                        );
                                    }
                                });
                            }
                            None => {
                                // Sender dropped — handler is gone, session closed
                                tracing::info!(
                                    "Remote forward event channel closed for remote port {}",
                                    remote_port
                                );
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Stopping remote forward on remote port {}", remote_port);
                        break;
                    }
                }
            }

            *status_clone.lock() = TunnelStatus::Stopped;
        });

        self.tunnels.push(TunnelHandle {
            id,
            status,
            bytes_sent,
            bytes_received,
            shutdown_tx,
        });

        Ok(id)
    }

    /// Start a dynamic SOCKS5 port forward (SSH -D equivalent).
    /// Binds a local SOCKS5 proxy on `local_host:local_port`; each accepted
    /// connection performs a SOCKS5 handshake and CONNECT request, then is
    /// tunneled to the requested target through an SSH `direct-tcpip` channel.
    pub async fn start_socks_forward(
        &mut self,
        handle: SharedHandle,
        local_host: String,
        local_port: u16,
    ) -> crate::Result<Uuid> {
        if !Self::check_port_available(local_port).await {
            return Err(SshError::PortInUse(local_port));
        }

        let id = Uuid::new_v4();
        let status = Arc::new(ParkingMutex::new(TunnelStatus::Active));
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let bytes_received = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        let status_clone = status.clone();
        let bytes_sent_clone = bytes_sent.clone();
        let bytes_received_clone = bytes_received.clone();

        tokio::spawn(async move {
            let bind_addr = format!("{}:{}", local_host, local_port);
            let listener = match TcpListener::bind(&bind_addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind SOCKS5 listener on {}: {}", bind_addr, e);
                    *status_clone.lock() = TunnelStatus::Error;
                    return;
                }
            };

            tracing::info!("Dynamic forward: SOCKS5 proxy on {}", bind_addr);

            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, addr)) => {
                                tracing::debug!("Accepted SOCKS5 connection from {}", addr);
                                let handle = handle.clone();
                                let bs = bytes_sent_clone.clone();
                                let br = bytes_received_clone.clone();

                                tokio::spawn(async move {
                                    if let Err(e) =
                                        handle_socks_connection(handle, stream, bs, br).await
                                    {
                                        tracing::error!("SOCKS5 connection error: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("SOCKS5 accept error: {}", e);
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Stopping SOCKS5 forward on {}", bind_addr);
                        break;
                    }
                }
            }

            *status_clone.lock() = TunnelStatus::Stopped;
        });

        self.tunnels.push(TunnelHandle {
            id,
            status,
            bytes_sent,
            bytes_received,
            shutdown_tx,
        });

        Ok(id)
    }

    /// Stop all active tunnels.
    pub fn stop_all(&self) {
        for tunnel in &self.tunnels {
            tunnel.stop();
        }
    }

    /// Get count of active tunnels.
    pub fn active_count(&self) -> usize {
        self.tunnels.iter().filter(|t| t.is_active()).count()
    }

    /// Remove stopped tunnels from the list.
    pub fn cleanup(&mut self) {
        self.tunnels.retain(|t| t.is_active());
    }

    /// Get a reference to all tunnel handles.
    pub fn tunnels(&self) -> &[TunnelHandle] {
        &self.tunnels
    }

    /// Find a tunnel by ID.
    pub fn get_tunnel(&self, id: &Uuid) -> Option<&TunnelHandle> {
        self.tunnels.iter().find(|t| t.id == *id)
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle a single forwarded TCP connection for local port forwarding.
/// Opens a direct-tcpip channel through SSH and performs bidirectional data copy
/// using the channel's into_stream() for clean AsyncRead/AsyncWrite integration.
async fn handle_local_forward_connection(
    handle: SharedHandle,
    tcp_stream: tokio::net::TcpStream,
    remote_host: &str,
    remote_port: u16,
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    // Open a direct-tcpip channel to the remote target
    let channel = {
        let h = handle.lock().await;
        h.channel_open_direct_tcpip(
            remote_host,
            remote_port as u32,
            "127.0.0.1", // originator address
            0,           // originator port
        )
        .await?
    };

    // Convert SSH channel into an AsyncRead + AsyncWrite stream
    let ssh_stream = channel.into_stream();
    let (mut ssh_read, mut ssh_write) = tokio::io::split(ssh_stream);
    let (mut tcp_read, mut tcp_write) = tokio::io::split(tcp_stream);

    // Spawn TCP -> SSH copy
    let bs = bytes_sent;
    let tcp_to_ssh = tokio::spawn(async move {
        let mut buf = vec![0u8; 32768];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    bs.fetch_add(n as u64, Ordering::Relaxed);
                    if ssh_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Spawn SSH -> TCP copy
    let br = bytes_received;
    let ssh_to_tcp = tokio::spawn(async move {
        let mut buf = vec![0u8; 32768];
        loop {
            match ssh_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    br.fetch_add(n as u64, Ordering::Relaxed);
                    if tcp_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let _ = tokio::join!(tcp_to_ssh, ssh_to_tcp);
    Ok(())
}

/// Handle a single server-initiated forwarded-tcpip connection for remote port forwarding.
///
/// The SSH server has opened `channel` because a client connected to the remote
/// forwarded port. We connect to the local target (`local_host:local_port`) and
/// perform bidirectional data copy between the SSH channel and the local TCP
/// connection, tracking bytes sent and received via atomic counters.
async fn handle_remote_forward_connection(
    channel: russh::Channel<russh::client::Msg>,
    local_host: &str,
    local_port: u16,
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    // Connect to the local target
    let tcp_stream = TcpStream::connect(format!("{}:{}", local_host, local_port))
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to local target {}:{}: {}",
                local_host,
                local_port,
                e
            )
        })?;

    tracing::debug!(
        "Connected to local target {}:{} for remote forward",
        local_host,
        local_port
    );

    // Convert SSH channel into an AsyncRead + AsyncWrite stream
    let ssh_stream = channel.into_stream();
    let (mut ssh_read, mut ssh_write) = tokio::io::split(ssh_stream);
    let (mut tcp_read, mut tcp_write) = tokio::io::split(tcp_stream);

    // Spawn SSH -> TCP copy (data from remote client to local target)
    let br = bytes_received;
    let ssh_to_tcp = tokio::spawn(async move {
        let mut buf = vec![0u8; 32768];
        loop {
            match ssh_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    br.fetch_add(n as u64, Ordering::Relaxed);
                    if tcp_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Spawn TCP -> SSH copy (data from local target back to remote client)
    let bs = bytes_sent;
    let tcp_to_ssh = tokio::spawn(async move {
        let mut buf = vec![0u8; 32768];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    bs.fetch_add(n as u64, Ordering::Relaxed);
                    if ssh_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let _ = tokio::join!(ssh_to_tcp, tcp_to_ssh);
    Ok(())
}

// SOCKS5 protocol constants (RFC 1928).
const SOCKS5_VERSION: u8 = 0x05;
const SOCKS5_AUTH_NONE: u8 = 0x00;
const SOCKS5_AUTH_NO_ACCEPTABLE: u8 = 0xFF;
const SOCKS5_CMD_CONNECT: u8 = 0x01;
const SOCKS5_ATYP_IPV4: u8 = 0x01;
const SOCKS5_ATYP_DOMAIN: u8 = 0x03;
const SOCKS5_ATYP_IPV6: u8 = 0x04;
// Reply codes
const SOCKS5_REP_SUCCESS: u8 = 0x00;
const SOCKS5_REP_GENERAL_FAILURE: u8 = 0x01;
const SOCKS5_REP_HOST_UNREACHABLE: u8 = 0x04;
const SOCKS5_REP_CMD_NOT_SUPPORTED: u8 = 0x07;
const SOCKS5_REP_ATYP_NOT_SUPPORTED: u8 = 0x08;

/// Write a SOCKS5 reply with the given reply code and a bound address of
/// `0.0.0.0:0` (BND.ADDR/BND.PORT are not meaningful for our CONNECT tunnel).
async fn send_socks_reply<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    reply: u8,
) -> std::io::Result<()> {
    // VER, REP, RSV, ATYP(IPv4), BND.ADDR(0.0.0.0), BND.PORT(0)
    let resp = [
        SOCKS5_VERSION,
        reply,
        0x00,
        SOCKS5_ATYP_IPV4,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    writer.write_all(&resp).await?;
    writer.flush().await
}

/// Handle a single SOCKS5 client connection: perform method negotiation and the
/// CONNECT request, open an SSH `direct-tcpip` channel to the requested target,
/// and pump bytes bidirectionally (mirroring `handle_local_forward_connection`).
async fn handle_socks_connection(
    handle: SharedHandle,
    mut tcp_stream: TcpStream,
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    // --- Method negotiation ---
    // Client greeting: VER, NMETHODS, METHODS...
    let mut head = [0u8; 2];
    tcp_stream.read_exact(&mut head).await?;
    if head[0] != SOCKS5_VERSION {
        anyhow::bail!("Unsupported SOCKS version: {}", head[0]);
    }
    let nmethods = head[1] as usize;
    let mut methods = vec![0u8; nmethods];
    tcp_stream.read_exact(&mut methods).await?;

    if !methods.contains(&SOCKS5_AUTH_NONE) {
        // No acceptable methods.
        tcp_stream
            .write_all(&[SOCKS5_VERSION, SOCKS5_AUTH_NO_ACCEPTABLE])
            .await?;
        anyhow::bail!("Client offered no supported SOCKS5 auth method");
    }
    // Select "no authentication required".
    tcp_stream
        .write_all(&[SOCKS5_VERSION, SOCKS5_AUTH_NONE])
        .await?;

    // --- Request ---
    // VER, CMD, RSV, ATYP
    let mut req = [0u8; 4];
    tcp_stream.read_exact(&mut req).await?;
    if req[0] != SOCKS5_VERSION {
        send_socks_reply(&mut tcp_stream, SOCKS5_REP_GENERAL_FAILURE).await?;
        anyhow::bail!("Unsupported SOCKS version in request: {}", req[0]);
    }
    let cmd = req[1];
    let atyp = req[3];

    if cmd != SOCKS5_CMD_CONNECT {
        // BIND (0x02) and UDP ASSOCIATE (0x03) are not supported.
        send_socks_reply(&mut tcp_stream, SOCKS5_REP_CMD_NOT_SUPPORTED).await?;
        anyhow::bail!("Unsupported SOCKS5 command: {}", cmd);
    }

    // Parse the target address.
    let target_host = match atyp {
        SOCKS5_ATYP_IPV4 => {
            let mut addr = [0u8; 4];
            tcp_stream.read_exact(&mut addr).await?;
            std::net::Ipv4Addr::from(addr).to_string()
        }
        SOCKS5_ATYP_IPV6 => {
            let mut addr = [0u8; 16];
            tcp_stream.read_exact(&mut addr).await?;
            std::net::Ipv6Addr::from(addr).to_string()
        }
        SOCKS5_ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            tcp_stream.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            tcp_stream.read_exact(&mut domain).await?;
            String::from_utf8(domain)
                .map_err(|e| anyhow::anyhow!("Invalid SOCKS5 domain name: {}", e))?
        }
        other => {
            send_socks_reply(&mut tcp_stream, SOCKS5_REP_ATYP_NOT_SUPPORTED).await?;
            anyhow::bail!("Unsupported SOCKS5 address type: {}", other);
        }
    };

    let mut port_buf = [0u8; 2];
    tcp_stream.read_exact(&mut port_buf).await?;
    let target_port = u16::from_be_bytes(port_buf);

    tracing::debug!("SOCKS5 CONNECT -> {}:{}", target_host, target_port);

    // Open a direct-tcpip channel to the requested target through SSH.
    let channel = {
        let h = handle.lock().await;
        h.channel_open_direct_tcpip(
            target_host.clone(),
            target_port as u32,
            "127.0.0.1", // originator address
            0,           // originator port
        )
        .await
    };

    let channel = match channel {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                "SOCKS5: failed to open channel to {}:{}: {}",
                target_host,
                target_port,
                e
            );
            send_socks_reply(&mut tcp_stream, SOCKS5_REP_HOST_UNREACHABLE).await?;
            return Ok(());
        }
    };

    // Tell the client the connection succeeded.
    send_socks_reply(&mut tcp_stream, SOCKS5_REP_SUCCESS).await?;

    // Convert SSH channel into an AsyncRead + AsyncWrite stream and pump bytes.
    let ssh_stream = channel.into_stream();
    let (mut ssh_read, mut ssh_write) = tokio::io::split(ssh_stream);
    let (mut tcp_read, mut tcp_write) = tokio::io::split(tcp_stream);

    // Spawn TCP -> SSH copy
    let bs = bytes_sent;
    let tcp_to_ssh = tokio::spawn(async move {
        let mut buf = vec![0u8; 32768];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    bs.fetch_add(n as u64, Ordering::Relaxed);
                    if ssh_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Spawn SSH -> TCP copy
    let br = bytes_received;
    let ssh_to_tcp = tokio::spawn(async move {
        let mut buf = vec![0u8; 32768];
        loop {
            match ssh_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    br.fetch_add(n as u64, Ordering::Relaxed);
                    if tcp_write.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let _ = tokio::join!(tcp_to_ssh, ssh_to_tcp);
    Ok(())
}
