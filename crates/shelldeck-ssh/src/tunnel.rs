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

            let mut connections = tokio::task::JoinSet::new();
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

                                connections.spawn(async move {
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
                    result = connections.join_next(), if !connections.is_empty() => {
                        if let Some(Err(e)) = result {
                            tracing::debug!("Local forward connection task ended: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Stopping local forward on port {}", local_port);
                        break;
                    }
                }
            }

            connections.abort_all();
            while connections.join_next().await.is_some() {}
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
        // Request remote forwarding from the server.
        {
            let h = handle.lock().await;
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
            let mut connections = tokio::task::JoinSet::new();
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

                                connections.spawn(async move {
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
                    result = connections.join_next(), if !connections.is_empty() => {
                        if let Some(Err(e)) = result {
                            tracing::debug!("Remote forward connection task ended: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Stopping remote forward on remote port {}", remote_port);
                        break;
                    }
                }
            }

            connections.abort_all();
            while connections.join_next().await.is_some() {}
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

            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, addr)) => {
                                tracing::debug!("Accepted SOCKS5 connection from {}", addr);
                                let handle = handle.clone();
                                let bs = bytes_sent_clone.clone();
                                let br = bytes_received_clone.clone();

                                connections.spawn(async move {
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
                    result = connections.join_next(), if !connections.is_empty() => {
                        if let Some(Err(e)) = result {
                            tracing::debug!("SOCKS5 connection task ended: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Stopping SOCKS5 forward on {}", bind_addr);
                        break;
                    }
                }
            }

            connections.abort_all();
            while connections.join_next().await.is_some() {}
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

    // TCP -> SSH copy
    let bs = bytes_sent;
    let tcp_to_ssh = async move {
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
    };

    // SSH -> TCP copy
    let br = bytes_received;
    let ssh_to_tcp = async move {
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
    };

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

    // SSH -> TCP copy (data from remote client to local target)
    let br = bytes_received;
    let ssh_to_tcp = async move {
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
    };

    // TCP -> SSH copy (data from local target back to remote client)
    let bs = bytes_sent;
    let tcp_to_ssh = async move {
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
    };

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

    // TCP -> SSH copy
    let bs = bytes_sent;
    let tcp_to_ssh = async move {
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
    };

    // SSH -> TCP copy
    let br = bytes_received;
    let ssh_to_tcp = async move {
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
    };

    let _ = tokio::join!(tcp_to_ssh, ssh_to_tcp);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TunnelManager, TunnelStatus};
    use crate::handler::{ClientHandler, ForwardedTcpIpEvent};
    use crate::session::SharedHandle;
    use russh::keys::{ssh_key::Algorithm, PrivateKey};
    use russh::server::{self, Auth, Msg, Session};
    use russh::Channel;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{mpsc, Mutex};
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout};

    #[derive(Debug, PartialEq, Eq)]
    struct DirectTcpIpRequest {
        host: String,
        port: u32,
    }

    struct EchoServer {
        requests: mpsc::UnboundedSender<DirectTcpIpRequest>,
    }

    impl server::Handler for EchoServer {
        type Error = anyhow::Error;

        async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_direct_tcpip(
            &mut self,
            channel: Channel<Msg>,
            host_to_connect: &str,
            port_to_connect: u32,
            _originator_address: &str,
            _originator_port: u32,
            _session: &mut Session,
        ) -> Result<bool, Self::Error> {
            let _ = self.requests.send(DirectTcpIpRequest {
                host: host_to_connect.to_owned(),
                port: port_to_connect,
            });

            tokio::spawn(async move {
                let stream = channel.into_stream();
                let (mut reader, mut writer) = tokio::io::split(stream);
                let _ = tokio::io::copy(&mut reader, &mut writer).await;
            });

            Ok(true)
        }
    }

    /// Jump-free reverse-forward server: records `tcpip_forward` requests and
    /// hands its own [`server::Handle`] back to the test so the test can play
    /// the remote side and open a forwarded-tcpip channel on demand.
    struct RemoteForwardServer {
        forward_requests: mpsc::UnboundedSender<(String, u32)>,
        server_handles: mpsc::UnboundedSender<server::Handle>,
    }

    impl server::Handler for RemoteForwardServer {
        type Error = anyhow::Error;

        async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn tcpip_forward(
            &mut self,
            address: &str,
            port: &mut u32,
            session: &mut Session,
        ) -> Result<bool, Self::Error> {
            let _ = self.forward_requests.send((address.to_owned(), *port));
            let _ = self.server_handles.send(session.handle());
            Ok(true)
        }
    }

    #[allow(clippy::type_complexity)]
    async fn start_remote_forward_server() -> (
        SharedHandle,
        mpsc::UnboundedReceiver<(String, u32)>,
        mpsc::UnboundedReceiver<server::Handle>,
        mpsc::UnboundedReceiver<ForwardedTcpIpEvent>,
        JoinHandle<()>,
    ) {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let (forward_tx, forward_rx) = mpsc::unbounded_channel();
        let (handles_tx, handles_rx) = mpsc::unbounded_channel();
        let server_config = server::Config {
            inactivity_timeout: None,
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(1)),
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .expect("generate in-memory SSH host key")],
            ..Default::default()
        };

        let server_task = tokio::spawn(async move {
            let running = server::run_stream(
                Arc::new(server_config),
                server_stream,
                RemoteForwardServer {
                    forward_requests: forward_tx,
                    server_handles: handles_tx,
                },
            )
            .await
            .expect("start in-memory SSH remote-forward server");
            let _ = running.await;
        });

        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (forwarded_tx, forwarded_rx) = mpsc::unbounded_channel();
        let handler = ClientHandler::new_trusting_server_key_for_test(event_tx, forwarded_tx);
        let mut handle = russh::client::connect_stream(
            Arc::new(russh::client::Config::default()),
            client_stream,
            handler,
        )
        .await
        .expect("connect to in-memory SSH remote-forward server");
        assert!(handle
            .authenticate_none("shelldeck-test")
            .await
            .expect("authenticate in-memory SSH client")
            .success());

        (
            Arc::new(Mutex::new(handle)),
            forward_rx,
            handles_rx,
            forwarded_rx,
            server_task,
        )
    }

    /// Real loopback echo listener standing in for the local target of a
    /// reverse forward.
    async fn start_local_echo_target() -> (u16, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local echo target");
        let port = listener.local_addr().expect("read local address").port();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut reader, mut writer) = tokio::io::split(stream);
                    let _ = tokio::io::copy(&mut reader, &mut writer).await;
                });
            }
        });
        (port, task)
    }

    async fn start_echo_server() -> (
        SharedHandle,
        mpsc::UnboundedReceiver<DirectTcpIpRequest>,
        JoinHandle<()>,
    ) {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let server_config = server::Config {
            inactivity_timeout: None,
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(1)),
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .expect("generate in-memory SSH host key")],
            ..Default::default()
        };

        let server_task = tokio::spawn(async move {
            let running = server::run_stream(
                Arc::new(server_config),
                server_stream,
                EchoServer {
                    requests: request_tx,
                },
            )
            .await
            .expect("start in-memory SSH echo server");
            let _ = running.await;
        });

        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (forwarded_tx, _forwarded_rx) = mpsc::unbounded_channel();
        let handler = ClientHandler::new_trusting_server_key_for_test(event_tx, forwarded_tx);
        let mut handle = russh::client::connect_stream(
            Arc::new(russh::client::Config::default()),
            client_stream,
            handler,
        )
        .await
        .expect("connect to in-memory SSH echo server");
        assert!(handle
            .authenticate_none("shelldeck-test")
            .await
            .expect("authenticate in-memory SSH client")
            .success());

        (Arc::new(Mutex::new(handle)), request_rx, server_task)
    }

    async fn unused_local_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve an ephemeral port");
        listener.local_addr().expect("read local address").port()
    }

    async fn connect_when_ready(port: u16) -> TcpStream {
        timeout(Duration::from_secs(2), async move {
            loop {
                match TcpStream::connect(("127.0.0.1", port)).await {
                    Ok(stream) => return stream,
                    Err(_) => sleep(Duration::from_millis(5)).await,
                }
            }
        })
        .await
        .expect("tunnel listener did not become ready")
    }

    async fn wait_until_stopped(manager: &TunnelManager, id: uuid::Uuid) {
        timeout(Duration::from_secs(2), async {
            loop {
                if manager
                    .get_tunnel(&id)
                    .is_some_and(|tunnel| *tunnel.status.lock() == TunnelStatus::Stopped)
                {
                    return;
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("tunnel did not stop");
    }

    async fn negotiate_no_auth(stream: &mut TcpStream) {
        stream
            .write_all(&[0x05, 0x01, 0x00])
            .await
            .expect("write SOCKS5 greeting");
        let mut response = [0_u8; 2];
        stream
            .read_exact(&mut response)
            .await
            .expect("read SOCKS5 greeting response");
        assert_eq!(response, [0x05, 0x00]);
    }

    // SDTEST-562, SDTEST-564, SDTEST-568, SDTEST-569
    #[tokio::test]
    async fn local_forward_echoes_tracks_bytes_and_drains_on_stop() {
        let (handle, mut requests, server_task) = start_echo_server().await;
        let local_port = unused_local_port().await;
        let mut manager = TunnelManager::new();
        let id = manager
            .start_local_forward(handle, local_port, "echo.internal".to_owned(), 4242)
            .await
            .expect("start local forward");

        let mut client = connect_when_ready(local_port).await;
        client.write_all(b"shelldeck").await.expect("write tunnel");
        let mut echoed = [0_u8; 9];
        client
            .read_exact(&mut echoed)
            .await
            .expect("read tunnel echo");
        assert_eq!(&echoed, b"shelldeck");
        assert_eq!(
            timeout(Duration::from_secs(2), requests.recv())
                .await
                .expect("direct-tcpip request timed out")
                .expect("direct-tcpip request channel closed"),
            DirectTcpIpRequest {
                host: "echo.internal".to_owned(),
                port: 4242,
            }
        );
        assert_eq!(
            manager
                .get_tunnel(&id)
                .expect("tunnel handle")
                .total_bytes(),
            (9, 9)
        );

        manager.get_tunnel(&id).expect("tunnel handle").stop();
        wait_until_stopped(&manager, id).await;

        let mut one = [0_u8; 1];
        assert_eq!(
            timeout(Duration::from_secs(1), client.read(&mut one))
                .await
                .expect("active tunnel connection remained open")
                .expect("read closed tunnel connection"),
            0,
            "stopping a tunnel must close already accepted connections"
        );

        manager.cleanup();
        assert!(manager.get_tunnel(&id).is_none());
        server_task.abort();
    }

    // SDTEST-565
    #[tokio::test]
    async fn remote_forward_requests_the_port_and_routes_connections_to_the_local_target() {
        let (handle, mut forward_requests, mut server_handles, forwarded_rx, server_task) =
            start_remote_forward_server().await;
        let (local_port, echo_task) = start_local_echo_target().await;
        let mut manager = TunnelManager::new();

        // Unlike the local and SOCKS forwards, `start_remote_forward` does not
        // keep the client handle: its channels are opened by the server, not by
        // the tunnel. Production holds it through the owning `SshSession`, so
        // the test has to hold it too or the transport closes underneath us.
        let session_handle = handle.clone();
        let id = manager
            .start_remote_forward(
                handle,
                8443,
                "127.0.0.1".to_owned(),
                local_port,
                forwarded_rx,
            )
            .await
            .expect("start remote forward");

        // The server must be asked to listen on the *remote* port, on every
        // interface — this is what makes it a reverse forward.
        assert_eq!(
            timeout(Duration::from_secs(2), forward_requests.recv())
                .await
                .expect("tcpip-forward request timed out")
                .expect("tcpip-forward request channel closed"),
            ("0.0.0.0".to_owned(), 8443),
        );

        // Play the remote side: a client hit the forwarded port, so the server
        // opens the channel back to us.
        let server_handle = timeout(Duration::from_secs(2), server_handles.recv())
            .await
            .expect("server handle timed out")
            .expect("server handle channel closed");
        let channel = server_handle
            .channel_open_forwarded_tcpip("0.0.0.0", 8443, "203.0.113.7", 54321)
            .await
            .expect("open forwarded-tcpip channel");

        let mut remote = channel.into_stream();
        remote
            .write_all(b"reverse")
            .await
            .expect("write through the reverse forward");
        let mut echoed = [0_u8; 7];
        timeout(Duration::from_secs(2), remote.read_exact(&mut echoed))
            .await
            .expect("local target never answered")
            .expect("read the local target echo");
        assert_eq!(&echoed, b"reverse");

        // Counters are directional: received is remote → local target, sent is
        // the local target's answer travelling back out.
        assert_eq!(
            manager
                .get_tunnel(&id)
                .expect("tunnel handle")
                .total_bytes(),
            (7, 7)
        );

        manager.get_tunnel(&id).expect("tunnel handle").stop();
        wait_until_stopped(&manager, id).await;

        let mut one = [0_u8; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), remote.read(&mut one))
                .await
                .expect("active reverse-forward connection remained open")
                .expect("read closed reverse-forward connection"),
            0,
            "stopping a reverse forward must close already routed connections"
        );

        manager.cleanup();
        assert!(manager.get_tunnel(&id).is_none());
        drop(session_handle);
        echo_task.abort();
        server_task.abort();
    }

    // SDTEST-566
    #[tokio::test]
    async fn socks5_connect_echoes_and_rejects_bind_and_udp_associate() {
        let (handle, mut requests, server_task) = start_echo_server().await;
        let local_port = unused_local_port().await;
        let mut manager = TunnelManager::new();
        let id = manager
            .start_socks_forward(handle, "127.0.0.1".to_owned(), local_port)
            .await
            .expect("start SOCKS5 forward");

        let mut client = connect_when_ready(local_port).await;
        negotiate_no_auth(&mut client).await;
        let host = b"echo.internal";
        let mut connect = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
        connect.extend_from_slice(host);
        connect.extend_from_slice(&4242_u16.to_be_bytes());
        client
            .write_all(&connect)
            .await
            .expect("write SOCKS5 CONNECT");
        let mut reply = [0_u8; 10];
        client
            .read_exact(&mut reply)
            .await
            .expect("read SOCKS5 CONNECT reply");
        assert_eq!(reply[0], 0x05);
        assert_eq!(reply[1], 0x00);
        assert_eq!(
            timeout(Duration::from_secs(2), requests.recv())
                .await
                .expect("SOCKS direct-tcpip request timed out")
                .expect("SOCKS direct-tcpip request channel closed"),
            DirectTcpIpRequest {
                host: "echo.internal".to_owned(),
                port: 4242,
            }
        );

        client.write_all(b"proxy").await.expect("write SOCKS echo");
        let mut echoed = [0_u8; 5];
        client
            .read_exact(&mut echoed)
            .await
            .expect("read SOCKS echo");
        assert_eq!(&echoed, b"proxy");

        for command in [0x02_u8, 0x03_u8] {
            let mut rejected = connect_when_ready(local_port).await;
            negotiate_no_auth(&mut rejected).await;
            rejected
                .write_all(&[0x05, command, 0x00, 0x01, 127, 0, 0, 1, 0, 80])
                .await
                .expect("write unsupported SOCKS command");
            let mut rejection = [0_u8; 10];
            rejected
                .read_exact(&mut rejection)
                .await
                .expect("read unsupported SOCKS command reply");
            assert_eq!(rejection[1], 0x07);
        }

        assert!(requests.try_recv().is_err());
        assert_eq!(
            manager.get_tunnel(&id).expect("SOCKS handle").total_bytes(),
            (5, 5)
        );
        manager.get_tunnel(&id).expect("SOCKS handle").stop();
        wait_until_stopped(&manager, id).await;
        let mut one = [0_u8; 1];
        assert_eq!(
            timeout(Duration::from_secs(1), client.read(&mut one))
                .await
                .expect("SOCKS connection remained open")
                .expect("read closed SOCKS connection"),
            0
        );

        manager.cleanup();
        assert!(manager.get_tunnel(&id).is_none());
        server_task.abort();
    }

    // SDTEST-561, SDTEST-563
    #[tokio::test]
    async fn port_availability_and_prebound_start_failure_are_reported() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind occupied test port");
        let occupied_port = listener.local_addr().expect("occupied address").port();
        assert!(!TunnelManager::check_port_available(occupied_port).await);

        let free_port = unused_local_port().await;
        assert!(TunnelManager::check_port_available(free_port).await);

        let (handle, _requests, server_task) = start_echo_server().await;
        let mut manager = TunnelManager::new();
        let error = manager
            .start_local_forward(handle, occupied_port, "echo.internal".to_owned(), 4242)
            .await
            .expect_err("prebound local port must fail");
        assert!(matches!(error, crate::SshError::PortInUse(port) if port == occupied_port));
        assert!(manager.tunnels().is_empty());

        server_task.abort();
    }

    // SDTEST-567
    #[tokio::test]
    async fn stop_all_closes_every_listener_and_active_connection() {
        let (handle, _requests, server_task) = start_echo_server().await;
        let first_port = unused_local_port().await;
        let second_port = unused_local_port().await;
        let mut manager = TunnelManager::new();
        let first_id = manager
            .start_local_forward(
                handle.clone(),
                first_port,
                "first.internal".to_owned(),
                1001,
            )
            .await
            .expect("start first tunnel");
        let second_id = manager
            .start_local_forward(handle, second_port, "second.internal".to_owned(), 1002)
            .await
            .expect("start second tunnel");
        let mut first = connect_when_ready(first_port).await;
        let mut second = connect_when_ready(second_port).await;
        first.write_all(b"a").await.expect("write first tunnel");
        second.write_all(b"b").await.expect("write second tunnel");
        let mut byte = [0_u8; 1];
        first
            .read_exact(&mut byte)
            .await
            .expect("read first tunnel");
        second
            .read_exact(&mut byte)
            .await
            .expect("read second tunnel");

        manager.stop_all();
        wait_until_stopped(&manager, first_id).await;
        wait_until_stopped(&manager, second_id).await;
        assert_eq!(manager.active_count(), 0);
        assert_eq!(
            timeout(Duration::from_secs(1), first.read(&mut byte))
                .await
                .expect("first tunnel remained open")
                .expect("read closed first tunnel"),
            0
        );
        assert_eq!(
            timeout(Duration::from_secs(1), second.read(&mut byte))
                .await
                .expect("second tunnel remained open")
                .expect("read closed second tunnel"),
            0
        );

        manager.cleanup();
        assert!(manager.tunnels().is_empty());
        server_task.abort();
    }
}
