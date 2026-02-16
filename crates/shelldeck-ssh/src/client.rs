use crate::handler::ClientHandler;
use crate::session::SshSession;
use crate::SshError;
use russh::client;
use shelldeck_core::models::{Connection, ConnectionSource, ConnectionStatus};
use std::path::Path;
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
        if let Some(ref proxy_jump) = connection.proxy_jump {
            // Take only the first hop if a comma-separated chain is specified.
            let first_hop = proxy_jump.split(',').next().unwrap_or(proxy_jump).trim();
            if first_hop.is_empty() || first_hop.eq_ignore_ascii_case("none") {
                // ProxyJump none means direct connection
                return self.connect_direct(connection).await;
            }
            tracing::info!(
                "Using ProxyJump '{}' to reach {}:{}",
                first_hop,
                connection.hostname,
                connection.port
            );
            self.connect_via_jump_host(first_hop, connection).await
        } else {
            self.connect_direct(connection).await
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
        let channel = {
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
            })?
        };

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
            // No user specified, use current user
            let user = std::env::var("USER")
                .or_else(|_| std::env::var("LOGNAME"))
                .unwrap_or_else(|_| "root".to_string());
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
            // Try default key locations
            let home = std::env::var("HOME").unwrap_or_default();
            let default_keys = [
                format!("{}/.ssh/id_ed25519", home),
                format!("{}/.ssh/id_rsa", home),
                format!("{}/.ssh/id_ecdsa", home),
            ];

            for key_path in &default_keys {
                let path = Path::new(key_path);
                if path.exists() {
                    match self.auth_with_key(handle, &connection.user, path).await {
                        Ok(()) => return Ok(()),
                        Err(e) => tracing::debug!("Key auth with {} failed: {}", key_path, e),
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
        let key_pair = match russh_keys::load_secret_key(key_path, None) {
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
                        russh_keys::load_secret_key(key_path, Some(&passphrase)).map_err(|e| {
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

        let auth_result = handle
            .authenticate_publickey(user, Arc::new(key_pair))
            .await
            .map_err(|e| SshError::AuthFailed(e.to_string()))?;

        if !auth_result {
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

        if !auth_result {
            tracing::warn!("Password authentication rejected for {}@{}", user, hostname);
            return Err(SshError::AuthFailed("Password rejected by server".into()));
        }

        tracing::info!("Authenticated with password for {}@{}", user, hostname);
        Ok(())
    }
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

    #[test]
    fn test_parse_jump_spec_identity_file_is_none() {
        // Jump host connections don't carry identity files from the spec string;
        // they rely on the default key probe in authenticate().
        let conn = SshClient::parse_jump_spec("root@10.0.0.1:22").unwrap();
        assert!(conn.identity_file.is_none());
        assert!(conn.proxy_jump.is_none());
    }
}
