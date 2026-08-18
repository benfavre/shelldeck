use crate::handler::ClientHandler;
use crate::session::SshSession;
use crate::SshError;
use russh::client;
use russh::keys::{Algorithm, PrivateKeyWithHashAlg};
use russh::Channel;
use shelldeck_core::models::{Connection, ConnectionSource, ConnectionStatus};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct SshClient {
    config: Arc<client::Config>,
}

impl SshClient {
    pub fn new() -> Self {
        let config = client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(30)),
            keepalive_interval: Some(std::time::Duration::from_secs(15)),
            keepalive_max: 3,
            ..Default::default()
        };

        Self {
            config: Arc::new(config),
        }
    }

    /// Connect to a remote host and authenticate.
    ///
    /// If the connection has a `proxy_jump` set, the client will first connect
    /// to the jump host, open a `direct-tcpip` channel to the final target, and
    /// then establish the SSH session over that forwarded channel.
    pub async fn connect(&self, connection: &Connection) -> crate::Result<SshSession> {
        match connection.proxy_jump.as_deref().and_then(first_jump_hop) {
            Some(first_hop) => {
                tracing::info!(
                    "Using ProxyJump '{}' to reach {}:{}",
                    first_hop,
                    connection.hostname,
                    connection.port
                );
                self.connect_via_jump_host(first_hop, connection).await
            }
            None => self.connect_direct(connection).await,
        }
    }

    /// Establish a direct TCP connection to the host (no proxy).
    async fn connect_direct(&self, connection: &Connection) -> crate::Result<SshSession> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (forwarded_tcpip_tx, forwarded_tcpip_rx) = mpsc::unbounded_channel();
        let handler = ClientHandler::new(
            event_tx,
            forwarded_tcpip_tx,
            connection.hostname.clone(),
            connection.port,
        );

        let addr = format!("{}:{}", connection.hostname, connection.port);

        tracing::info!("Connecting to {}", addr);

        let mut handle = client::connect(self.config.clone(), &*addr, handler)
            .await
            .map_err(|e| SshError::ConnectionFailed(e.to_string()))?;

        // Authenticate
        self.authenticate(&mut handle, connection).await?;

        Ok(SshSession::new(
            connection.id,
            handle,
            event_rx,
            forwarded_tcpip_rx,
        ))
    }

    /// Connect to the final target via a jump host using `direct-tcpip` forwarding.
    ///
    /// Steps:
    /// 1. Parse the jump host specifier into a `Connection`.
    /// 2. Connect & authenticate to the jump host (recursively, so chained jumps
    ///    could be supported in the future).
    /// 3. Open a `direct-tcpip` channel from the jump host to the final target.
    /// 4. Run the SSH handshake for the final target over that channel stream.
    /// 5. Return the final `SshSession`, which keeps the jump session alive internally.
    async fn connect_via_jump_host(
        &self,
        jump_spec: &str,
        target: &Connection,
    ) -> crate::Result<SshSession> {
        // --- 1. Build a Connection for the jump host ---
        let jump_connection = Self::parse_jump_spec(jump_spec)?;

        // --- 2. Connect to the jump host (may itself use a proxy) ---
        tracing::info!(
            "Connecting to jump host {}@{}:{}",
            jump_connection.user,
            jump_connection.hostname,
            jump_connection.port
        );
        let jump_session = self.connect_direct(&jump_connection).await.map_err(|e| {
            SshError::ConnectionFailed(format!(
                "Failed to connect to jump host '{}': {}",
                jump_spec, e
            ))
        })?;

        // --- 3. Open direct-tcpip channel through the jump host ---
        tracing::info!(
            "Opening direct-tcpip channel to {}:{} via jump host",
            target.hostname,
            target.port
        );
        let channel = Self::open_jump_channel(&jump_session, target).await?;

        // Convert the SSH channel into an AsyncRead + AsyncWrite stream
        let channel_stream = channel.into_stream();

        // --- 4. Run SSH handshake over the channel stream ---
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (forwarded_tcpip_tx, forwarded_tcpip_rx) = mpsc::unbounded_channel();
        let handler = ClientHandler::new(
            event_tx,
            forwarded_tcpip_tx,
            target.hostname.clone(),
            target.port,
        );

        tracing::info!(
            "Performing SSH handshake with {}:{} over jump channel",
            target.hostname,
            target.port
        );

        let mut handle = client::connect_stream(self.config.clone(), channel_stream, handler)
            .await
            .map_err(|e| {
                SshError::ConnectionFailed(format!("SSH handshake over jump channel failed: {}", e))
            })?;

        // --- 5. Authenticate on the final target ---
        self.authenticate(&mut handle, target).await?;

        tracing::info!(
            "Successfully connected to {}:{} via jump host '{}'",
            target.hostname,
            target.port,
            jump_spec
        );

        // Return session that keeps the jump session alive
        Ok(SshSession::new_with_jump(
            target.id,
            handle,
            event_rx,
            forwarded_tcpip_rx,
            jump_session,
        ))
    }

    /// Open the `direct-tcpip` channel that will carry the final SSH session
    /// through an already-established jump host session.
    ///
    /// Extracted from [`Self::connect_via_jump_host`] so the ProxyJump wiring can
    /// be proven against an in-memory jump server: the channel must be opened
    /// against the **target** host, never against the jump host itself.
    async fn open_jump_channel(
        jump_session: &SshSession,
        target: &Connection,
    ) -> crate::Result<Channel<client::Msg>> {
        let jump_handle = jump_session.shared_handle();
        let h = jump_handle.lock().await;
        h.channel_open_direct_tcpip(
            &target.hostname,
            target.port as u32,
            "127.0.0.1", // originator address
            0,           // originator port
        )
        .await
        .map_err(|e| {
            SshError::ConnectionFailed(format!(
                "Failed to open direct-tcpip channel to {}:{}: {}",
                target.hostname, target.port, e
            ))
        })
    }

    /// Parse a jump host specifier string into a `Connection`.
    ///
    /// Supported formats:
    /// - `host`                  -> current user @ host : 22
    /// - `host:port`             -> current user @ host : port
    /// - `user@host`             -> user @ host : 22
    /// - `user@host:port`        -> user @ host : port
    /// - `ssh://user@host:port`  -> user @ host : port
    fn parse_jump_spec(spec: &str) -> crate::Result<Connection> {
        let spec = spec.trim();

        // Strip optional ssh:// prefix
        let spec = spec.strip_prefix("ssh://").unwrap_or(spec);

        let (user, host_port) = if let Some(at_idx) = spec.find('@') {
            let user = &spec[..at_idx];
            let rest = &spec[at_idx + 1..];
            (user.to_string(), rest)
        } else {
            // No user specified — use the OS user (USER / LOGNAME on Unix,
            // USERNAME on Windows), with an explicit last-resort fallback
            // only when no environment can name the current user at all.
            let user =
                shelldeck_core::util::current_username().unwrap_or_else(|| "root".to_string());
            (user, spec)
        };

        let (hostname, port) = if let Some(colon_idx) = host_port.rfind(':') {
            // Could be host:port or just an IPv6 address
            let port_str = &host_port[colon_idx + 1..];
            if let Ok(port) = port_str.parse::<u16>() {
                let host = &host_port[..colon_idx];
                (host.to_string(), port)
            } else {
                // Not a valid port number, treat the whole thing as hostname
                (host_port.to_string(), 22)
            }
        } else {
            (host_port.to_string(), 22)
        };

        if hostname.is_empty() {
            return Err(SshError::ConnectionFailed(format!(
                "Invalid jump host specifier: empty hostname in '{}'",
                spec
            )));
        }

        Ok(Connection {
            id: Uuid::new_v4(),
            alias: format!("jump:{}", spec),
            hostname,
            port,
            user,
            identity_file: None,
            proxy_jump: None,
            group: None,
            tags: Vec::new(),
            auto_forwards: Vec::new(),
            auto_scripts: Vec::new(),
            source: ConnectionSource::Manual,
            forward_agent: false,
            site_id: None,
            site_label: None,
            status: ConnectionStatus::Disconnected,
        })
    }

    async fn authenticate(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        connection: &Connection,
    ) -> crate::Result<()> {
        // Try key-based auth with explicit key first
        if let Some(ref key_path) = connection.identity_file {
            match self.auth_with_key(handle, &connection.user, key_path).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::debug!(
                        "Explicit key auth with {} failed: {}",
                        key_path.display(),
                        e
                    );
                }
            }
        } else {
            // Try default key locations under the resolved home directory.
            for path in default_key_candidates(shelldeck_core::util::home_dir()) {
                if path.exists() {
                    match self.auth_with_key(handle, &connection.user, &path).await {
                        Ok(()) => return Ok(()),
                        Err(e) => {
                            tracing::debug!("Key auth with {} failed: {}", path.display(), e)
                        }
                    }
                }
            }
        }

        // Fallback: try password authentication from OS keychain
        match self
            .auth_with_password(handle, &connection.user, &connection.hostname)
            .await
        {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::debug!("Password auth fallback failed: {}", e);
            }
        }

        Err(SshError::AuthFailed(
            "No valid authentication method found".into(),
        ))
    }

    async fn auth_with_key(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        user: &str,
        key_path: &Path,
    ) -> crate::Result<()> {
        let key_pair = match russh::keys::load_secret_key(key_path, None) {
            Ok(kp) => kp,
            Err(unencrypted_err) => {
                // Key may be encrypted — try passphrase from keychain
                let path_str = key_path.to_string_lossy();
                tracing::debug!(
                    "Failed to load key {} without passphrase ({}), trying keychain",
                    path_str,
                    unencrypted_err
                );

                match shelldeck_core::config::keychain::get_key_passphrase(&path_str) {
                    Ok(Some(passphrase)) => {
                        russh::keys::load_secret_key(key_path, Some(&passphrase)).map_err(|e| {
                            tracing::warn!(
                                "Key {} failed with keychain passphrase: {}",
                                path_str,
                                e
                            );
                            SshError::Key(format!(
                                "Failed to load key {} (tried passphrase from keychain): {}",
                                path_str, e
                            ))
                        })?
                    }
                    Ok(None) => {
                        tracing::debug!(
                            "Key {} appears encrypted but no passphrase in keychain",
                            path_str
                        );
                        return Err(SshError::Key(format!(
                            "Failed to load key {}: {}",
                            path_str, unencrypted_err
                        )));
                    }
                    Err(kc_err) => {
                        tracing::warn!(
                            "Key {} encrypted, keychain lookup failed: {}",
                            path_str,
                            kc_err
                        );
                        return Err(SshError::Key(format!(
                            "Failed to load key {}: {}",
                            path_str, unencrypted_err
                        )));
                    }
                }
            }
        };

        let rsa_hash = if matches!(key_pair.algorithm(), Algorithm::Rsa { .. }) {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|e| SshError::AuthFailed(e.to_string()))?
                .flatten()
        } else {
            None
        };
        let auth_result = handle
            .authenticate_publickey(
                user,
                PrivateKeyWithHashAlg::new(Arc::new(key_pair), rsa_hash),
            )
            .await
            .map_err(|e| SshError::AuthFailed(e.to_string()))?;

        if !auth_result.success() {
            return Err(SshError::AuthFailed("Public key rejected".into()));
        }

        tracing::info!("Authenticated with key {}", key_path.display());
        Ok(())
    }

    async fn auth_with_password(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        user: &str,
        hostname: &str,
    ) -> crate::Result<()> {
        let password = match shelldeck_core::config::keychain::get_password(hostname, user) {
            Ok(Some(pw)) => pw,
            Ok(None) => {
                tracing::debug!("No password stored in keychain for {}@{}", user, hostname);
                return Err(SshError::AuthFailed("No password found in keychain".into()));
            }
            Err(e) => {
                tracing::warn!("Failed to access keychain for {}@{}: {}", user, hostname, e);
                return Err(SshError::AuthFailed(format!(
                    "Keychain access failed: {}",
                    e
                )));
            }
        };

        tracing::info!(
            "Attempting password authentication for {}@{}",
            user,
            hostname
        );

        let auth_result = handle
            .authenticate_password(user, &password)
            .await
            .map_err(|e| SshError::AuthFailed(e.to_string()))?;

        if !auth_result.success() {
            tracing::warn!("Password authentication rejected for {}@{}", user, hostname);
            return Err(SshError::AuthFailed("Password rejected by server".into()));
        }

        tracing::info!("Authenticated with password for {}@{}", user, hostname);
        Ok(())
    }
}

/// Select the hop to jump through from a `ProxyJump` value.
///
/// Returns `None` when the value disables proxying, which OpenSSH spells
/// `ProxyJump none`, and when the field is present but blank. Only the first
/// hop of a comma-separated chain is honored; chained jumps are not supported
/// yet and silently using the last hop would connect through the wrong host.
fn first_jump_hop(proxy_jump: &str) -> Option<&str> {
    let first_hop = proxy_jump.split(',').next().unwrap_or(proxy_jump).trim();
    if first_hop.is_empty() || first_hop.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(first_hop)
}

/// Default private-key candidates under `home`, in probe order
/// (ed25519 first, matching OpenSSH's modern preference).
///
/// Returns an empty list when no home directory could be resolved: the old
/// behavior built the paths off a raw `$HOME` string (empty on Windows),
/// probing fabricated root-level paths like `/.ssh/id_ed25519`. No home →
/// no default keys to try; explicit `identity_file` and the keychain
/// password fallback still apply.
fn default_key_candidates(home: Option<PathBuf>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    let ssh_dir = home.join(".ssh");
    ["id_ed25519", "id_rsa", "id_ecdsa"]
        .into_iter()
        .map(|name| ssh_dir.join(name))
        .collect()
}

impl Default for SshClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_jump_spec_host_only() {
        let conn = SshClient::parse_jump_spec("bastion.example.com").unwrap();
        assert_eq!(conn.hostname, "bastion.example.com");
        assert_eq!(conn.port, 22);
        // user falls back to the current $USER
    }

    #[test]
    fn test_parse_jump_spec_user_at_host() {
        let conn = SshClient::parse_jump_spec("admin@bastion.example.com").unwrap();
        assert_eq!(conn.hostname, "bastion.example.com");
        assert_eq!(conn.user, "admin");
        assert_eq!(conn.port, 22);
    }

    #[test]
    fn test_parse_jump_spec_user_at_host_port() {
        let conn = SshClient::parse_jump_spec("admin@bastion.example.com:2222").unwrap();
        assert_eq!(conn.hostname, "bastion.example.com");
        assert_eq!(conn.user, "admin");
        assert_eq!(conn.port, 2222);
    }

    #[test]
    fn test_parse_jump_spec_host_port() {
        let conn = SshClient::parse_jump_spec("bastion.example.com:2222").unwrap();
        assert_eq!(conn.hostname, "bastion.example.com");
        assert_eq!(conn.port, 2222);
    }

    #[test]
    fn test_parse_jump_spec_ssh_uri() {
        let conn = SshClient::parse_jump_spec("ssh://deploy@jump.internal:8022").unwrap();
        assert_eq!(conn.hostname, "jump.internal");
        assert_eq!(conn.user, "deploy");
        assert_eq!(conn.port, 8022);
    }

    #[test]
    fn test_parse_jump_spec_whitespace_trimmed() {
        let conn = SshClient::parse_jump_spec("  admin@bastion  ").unwrap();
        assert_eq!(conn.hostname, "bastion");
        assert_eq!(conn.user, "admin");
    }

    #[test]
    fn test_parse_jump_spec_empty_hostname_fails() {
        let result = SshClient::parse_jump_spec("admin@");
        assert!(result.is_err());
    }

    // Default key discovery builds paths off the resolved home with
    // platform-native joins, probing ed25519 → rsa → ecdsa in that order.
    #[test]
    fn default_key_candidates_are_under_home_ssh_in_probe_order() {
        let home = PathBuf::from("home-dir");
        let candidates = default_key_candidates(Some(home.clone()));
        let ssh_dir = home.join(".ssh");
        assert_eq!(
            candidates,
            vec![
                ssh_dir.join("id_ed25519"),
                ssh_dir.join("id_rsa"),
                ssh_dir.join("id_ecdsa"),
            ],
        );
    }

    // No resolvable home → no candidates. The old behavior formatted
    // `"{home}/.ssh/id_ed25519"` from a raw `$HOME` (empty on Windows) and
    // probed root-level `/.ssh/*` paths.
    #[test]
    fn default_key_candidates_empty_without_home_never_root_level() {
        assert!(default_key_candidates(None).is_empty());
    }

    #[test]
    fn test_parse_jump_spec_identity_file_is_none() {
        // Jump host connections don't carry identity files from the spec string;
        // they rely on the default key probe in authenticate().
        let conn = SshClient::parse_jump_spec("root@10.0.0.1:22").unwrap();
        assert!(conn.identity_file.is_none());
        assert!(conn.proxy_jump.is_none());
    }
}

/// ProxyJump transport proof.
///
/// Two real `russh` servers run over `tokio::io::duplex`: a bastion, and a
/// target whose whole SSH session is carried inside the bastion's
/// `direct-tcpip` channel. No socket is opened, and the user's `known_hosts`
/// is never read or written.
#[cfg(test)]
mod proxy_jump_transport_tests {
    use super::{first_jump_hop, SshClient};
    use crate::handler::{ClientHandler, ForwardedTcpIpEvent, SshEvent};
    use crate::session::SshSession;
    use russh::keys::{ssh_key::Algorithm, PrivateKey};
    use russh::server::{self, Auth, Msg, Session};
    use russh::{Channel, ChannelId};
    use shelldeck_core::models::Connection;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;
    use uuid::Uuid;

    /// Marker written by the *inner* server only. Seeing it in the exec output
    /// is what proves the session reached the target rather than the bastion.
    const TARGET_MARKER: &[u8] = b"reached-target-host\n";

    #[derive(Debug, PartialEq, Eq)]
    struct DirectTcpIpRequest {
        host: String,
        port: u32,
    }

    fn in_memory_server_config() -> server::Config {
        server::Config {
            inactivity_timeout: None,
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(1)),
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .expect("generate in-memory SSH host key")],
            ..Default::default()
        }
    }

    /// The final host. Answers any command with [`TARGET_MARKER`].
    struct TargetServer;

    impl server::Handler for TargetServer {
        type Error = anyhow::Error;

        async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            _channel: Channel<Msg>,
            _session: &mut Session,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            _command: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            session.data(channel, TARGET_MARKER.to_vec())?;
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            Ok(())
        }
    }

    /// The jump host. Records every `direct-tcpip` request and runs the target
    /// server inside the channel it just opened.
    struct BastionServer {
        requests: mpsc::UnboundedSender<DirectTcpIpRequest>,
    }

    impl server::Handler for BastionServer {
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
                let running = server::run_stream(
                    Arc::new(in_memory_server_config()),
                    channel.into_stream(),
                    TargetServer,
                )
                .await
                .expect("start in-memory SSH target server");
                let _ = running.await;
            });

            Ok(true)
        }
    }

    /// Authenticated session against the in-memory bastion, standing in for the
    /// jump session `connect_via_jump_host` builds with `connect_direct`.
    async fn start_jump_session() -> (
        SshSession,
        mpsc::UnboundedReceiver<DirectTcpIpRequest>,
        JoinHandle<()>,
    ) {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let (request_tx, request_rx) = mpsc::unbounded_channel();

        let server_task = tokio::spawn(async move {
            let running = server::run_stream(
                Arc::new(in_memory_server_config()),
                server_stream,
                BastionServer {
                    requests: request_tx,
                },
            )
            .await
            .expect("start in-memory SSH bastion server");
            let _ = running.await;
        });

        let (handle, event_rx, forwarded_rx) = handshake_in_memory(client_stream).await;
        (
            SshSession::new(Uuid::new_v4(), handle, event_rx, forwarded_rx),
            request_rx,
            server_task,
        )
    }

    /// Client half of an in-memory handshake: trusts the generated host key so
    /// nothing touches `~/.ssh/known_hosts`, and authenticates with `none`.
    async fn handshake_in_memory<S>(
        stream: S,
    ) -> (
        russh::client::Handle<ClientHandler>,
        mpsc::UnboundedReceiver<SshEvent>,
        mpsc::UnboundedReceiver<ForwardedTcpIpEvent>,
    )
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (forwarded_tx, forwarded_rx) = mpsc::unbounded_channel();
        let handler = ClientHandler::new_trusting_server_key_for_test(event_tx, forwarded_tx);
        let mut handle = russh::client::connect_stream(
            Arc::new(russh::client::Config::default()),
            stream,
            handler,
        )
        .await
        .expect("connect to in-memory SSH server");
        assert!(handle
            .authenticate_none("shelldeck-test")
            .await
            .expect("authenticate in-memory SSH client")
            .success());

        (handle, event_rx, forwarded_rx)
    }

    // SDTEST-528
    #[tokio::test]
    async fn jump_channel_targets_the_inner_host_and_carries_its_session() {
        let (jump_session, mut requests, server_task) = start_jump_session().await;

        let mut target = Connection::new_manual(
            "target".to_owned(),
            "inner.example.internal".to_owned(),
            "deploy".to_owned(),
        );
        target.port = 2022;

        let channel = timeout(
            Duration::from_secs(2),
            SshClient::open_jump_channel(&jump_session, &target),
        )
        .await
        .expect("direct-tcpip request timed out")
        .expect("open direct-tcpip channel through the jump host");

        // The bastion must be asked for the *target*, never for itself.
        let request = timeout(Duration::from_secs(2), requests.recv())
            .await
            .expect("direct-tcpip request not observed")
            .expect("bastion request channel closed");
        assert_eq!(
            request,
            DirectTcpIpRequest {
                host: "inner.example.internal".to_owned(),
                port: 2022,
            }
        );

        // A full second SSH session runs inside that channel, and the jump
        // session is moved into it so the tunnel stays open for its lifetime.
        let (inner_handle, inner_events, inner_forwarded) =
            handshake_in_memory(channel.into_stream()).await;
        let session = SshSession::new_with_jump(
            target.id,
            inner_handle,
            inner_events,
            inner_forwarded,
            jump_session,
        );

        let result = timeout(Duration::from_secs(2), session.exec("hostname"))
            .await
            .expect("exec over the jump channel timed out")
            .expect("exec over the jump channel failed");
        assert_eq!(result.stdout, TARGET_MARKER);
        assert_eq!(result.exit_code, Some(0));

        server_task.abort();
    }

    // SDTEST-530
    #[test]
    fn proxy_jump_none_or_blank_means_direct_and_a_chain_uses_its_first_hop() {
        assert_eq!(
            first_jump_hop("bastion.example.com"),
            Some("bastion.example.com")
        );
        assert_eq!(first_jump_hop("  admin@bastion  "), Some("admin@bastion"));
        assert_eq!(first_jump_hop("first@a,second@b"), Some("first@a"));
        assert_eq!(first_jump_hop("none"), None);
        assert_eq!(first_jump_hop("None"), None);
        assert_eq!(first_jump_hop(""), None);
        assert_eq!(first_jump_hop("   "), None);
    }
}
