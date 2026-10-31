//! A run-owned SSH bridge to ShellDeck's existing local browser capability.
//! The remote provider gets a temporary stdio adapter, not a desktop executable
//! path. Private launch data travels on SSH stdin, never shell source or argv.
use crate::{session::SshSession, SshError};
use base64::Engine;
use serde_json::json;
use shelldeck_core::agent_runtime::{AgentCommandSpec, AgentProvider};
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::copy_bidirectional,
    net::TcpStream,
    sync::{mpsc, oneshot},
    task::JoinSet,
};
use uuid::Uuid;

const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 8;
pub const LAUNCHER: &str = include_str!("browser_bridge.py");

pub struct BrowserBridgeConfig {
    run_id: Uuid,
    connection_id: Uuid,
    endpoint: SocketAddr,
    capability: String,
    revoked: oneshot::Receiver<()>,
}

impl std::fmt::Debug for BrowserBridgeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserBridgeConfig")
            .field("run_id", &self.run_id)
            .field("connection_id", &self.connection_id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// Retain alongside the local BrowserGrant. Dropping it releases the remote
/// listener and active forwarded streams without terminating the agent run.
pub struct BrowserBridgeLease {
    _sender: oneshot::Sender<()>,
}

impl BrowserBridgeConfig {
    pub fn new(
        run_id: Uuid,
        connection_id: Uuid,
        endpoint: &str,
        capability: String,
    ) -> crate::Result<(Self, BrowserBridgeLease)> {
        let endpoint: SocketAddr = endpoint.parse().map_err(|_| invalid_scope())?;
        if endpoint.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
            || endpoint.port() == 0
            || capability.len() != 64
            || !capability.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid_scope());
        }
        let (sender, revoked) = oneshot::channel();
        Ok((
            Self {
                run_id,
                connection_id,
                endpoint,
                capability,
                revoked,
            },
            BrowserBridgeLease { _sender: sender },
        ))
    }

    /// Cancel an in-flight connection attempt if the browser was closed before
    /// the remote provider started. No capability is transferred after expiry.
    pub async fn revoked(&mut self) {
        let _ = (&mut self.revoked).await;
    }
}

fn invalid_scope() -> SshError {
    SshError::Channel("Invalid or expired scoped remote browser connection".into())
}

pub fn launcher_command() -> String {
    let quoted = format!("'{}'", LAUNCHER.replace('\'', "'\\''"));
    format!("export PATH=\"$HOME/.local/bin:$HOME/bin:/usr/local/bin:/opt/homebrew/bin:$PATH\"; command -v python3 >/dev/null 2>&1 || {{ printf '%s\\n' 'ShellDeck browser sharing requires Python 3 on this SSH server' >&2; exit 126; }}; exec python3 -u -c {quoted}")
}

fn launch_input(
    provider: AgentProvider,
    spec: &AgentCommandSpec,
    remote_port: u16,
    capability: &str,
) -> crate::Result<Vec<u8>> {
    let provider = match provider {
        AgentProvider::Claude => "claude",
        AgentProvider::Codex => "codex",
        _ => return Err(invalid_scope()),
    };
    let input = serde_json::to_vec(&json!({
        "version":1, "provider":provider, "program":spec.program, "args":spec.args,
        "workdir":spec.cwd.to_str().ok_or_else(invalid_scope)?,
        "stdin":spec.stdin.as_ref().map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        "remove_env":spec.remove_env,
        "endpoint":format!("127.0.0.1:{remote_port}"), "capability":capability,
    })).map_err(|_| invalid_scope())?;
    if input.len() > 4 * 1024 * 1024 {
        return Err(SshError::Channel(
            "SSH browser launch request exceeds its input limit".into(),
        ));
    }
    Ok(input)
}

async fn cancel_listener(handle: &crate::session::SharedHandle, port: u16) {
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        handle
            .lock()
            .await
            .cancel_tcpip_forward("127.0.0.1", port as u32)
            .await
    })
    .await;
}

struct ListenerLease {
    handle: crate::session::SharedHandle,
    port: Option<u16>,
}
impl ListenerLease {
    async fn release(&mut self) {
        if let Some(port) = self.port {
            cancel_listener(&self.handle, port).await;
            self.port = None;
        }
    }
}
impl Drop for ListenerLease {
    fn drop(&mut self) {
        if let (Some(port), Ok(runtime)) = (self.port.take(), tokio::runtime::Handle::try_current())
        {
            let handle = self.handle.clone();
            runtime.spawn(async move {
                cancel_listener(&handle, port).await;
            });
        }
    }
}

/// Execute one already-admitted remote run. This uses its ordinary SSH session
/// (including ProxyJump) and a server-allocated IPv4 loopback reverse forward.
pub async fn run(
    session: &mut SshSession,
    run_id: Uuid,
    provider: AgentProvider,
    spec: &AgentCommandSpec,
    mut config: BrowserBridgeConfig,
    output: mpsc::UnboundedSender<Vec<u8>>,
    mut shutdown: mpsc::Receiver<()>,
) -> crate::Result<Option<u32>> {
    if config.run_id != run_id
        || config.connection_id != session.connection_id
        || !matches!(provider, AgentProvider::Claude | AgentProvider::Codex)
    {
        return Err(invalid_scope());
    }
    let mut incoming = session
        .take_forwarded_tcpip_rx()
        .ok_or_else(invalid_scope)?;
    let handle = session.shared_handle();
    let request = async {
        handle.lock().await.tcpip_forward("127.0.0.1", 0).await
            .map_err(|_| SshError::Tunnel("The SSH server refused browser forwarding; allow loopback remote port forwarding and retry".into()))
    };
    let allocated = tokio::select! {
        biased;
        _ = shutdown.recv() => return Ok(None),
        _ = &mut config.revoked => return Err(invalid_scope()),
        port = tokio::time::timeout(SETUP_TIMEOUT, request) => port.map_err(|_| SshError::Tunnel("SSH browser forwarding timed out".into()))??,
    };
    let port = match u16::try_from(allocated) {
        Ok(port) if port != 0 => port,
        _ => {
            return Err(SshError::Tunnel(
                "The SSH server did not allocate a browser port".into(),
            ))
        }
    };
    let mut listener = ListenerLease {
        handle: handle.clone(),
        port: Some(port),
    };
    let input = match launch_input(provider, spec, port, &config.capability) {
        Ok(input) => input,
        Err(error) => {
            listener.release().await;
            return Err(error);
        }
    };
    let command = launcher_command();
    let execution = session.exec_cancellable_with_input(&command, input, output, shutdown);
    tokio::pin!(execution);
    let mut streams = JoinSet::new();
    let mut forwarding = true;
    let result = loop {
        tokio::select! {
            biased;
            _ = &mut config.revoked, if forwarding => {
                forwarding = false;
                streams.abort_all();
                listener.release().await;
            }
            result = &mut execution => break result,
            event = incoming.recv(), if forwarding => {
                let Some(event) = event else { break Err(SshError::Channel("SSH browser connection ended".into())); };
                let loopback = |host: &str| host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
                if event.connected_port != u32::from(port) || !loopback(&event.connected_address)
                    || !loopback(&event.originator_address) || streams.len() >= MAX_CONNECTIONS
                { let _ = event.channel.close().await; continue; }
                let endpoint = config.endpoint;
                streams.spawn(async move {
                    let mut local = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(endpoint)).await
                        .map_err(|_| std::io::Error::other("browser connection timed out"))??;
                    let mut remote = event.channel.into_stream();
                    copy_bidirectional(&mut local, &mut remote).await
                });
            }
            _ = streams.join_next(), if !streams.is_empty() => {}
        }
    };
    streams.abort_all();
    while streams.join_next().await.is_some() {}
    listener.release().await;
    result
}

#[cfg(test)]
#[path = "browser_bridge_tests.rs"]
mod tests;
