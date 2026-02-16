use async_trait::async_trait;
use russh::client::{self, DisconnectReason, Msg, Session};
use russh::{Channel, ChannelId};
use russh_keys::key::PublicKey;
use russh_keys::PublicKeyBase64;
use tokio::sync::mpsc;

use crate::known_hosts::{self, KnownHostResult};

#[derive(Debug, Clone)]
pub enum SshEvent {
    Banner(String),
    Disconnected(String),
    Data {
        channel: ChannelId,
        data: Vec<u8>,
    },
    ExtendedData {
        channel: ChannelId,
        data: Vec<u8>,
        ext: u32,
    },
    ExitStatus {
        channel: ChannelId,
        code: u32,
    },
    ChannelEof(ChannelId),
    ChannelClose(ChannelId),
    Error(String),
}

/// Event sent when the server opens a channel for a reverse-forwarded TCP/IP connection.
/// This is the server-initiated counterpart of SSH -R: the remote side accepted an
/// incoming connection on a forwarded port and is relaying it back to us.
pub struct ForwardedTcpIpEvent {
    pub channel: Channel<Msg>,
    pub connected_address: String,
    pub connected_port: u32,
    pub originator_address: String,
    pub originator_port: u32,
}

pub struct ClientHandler {
    event_tx: mpsc::UnboundedSender<SshEvent>,
    forwarded_tcpip_tx: mpsc::UnboundedSender<ForwardedTcpIpEvent>,
    hostname: String,
    port: u16,
}

impl ClientHandler {
    pub fn new(
        event_tx: mpsc::UnboundedSender<SshEvent>,
        forwarded_tcpip_tx: mpsc::UnboundedSender<ForwardedTcpIpEvent>,
        hostname: String,
        port: u16,
    ) -> Self {
        Self {
            event_tx,
            forwarded_tcpip_tx,
            hostname,
            port,
        }
    }

    fn send_event(&self, event: SshEvent) {
        let _ = self.event_tx.send(event);
    }
}

#[async_trait]
impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let key_type = server_public_key.name();
        let key_base64 = server_public_key.public_key_base64();

        match known_hosts::check_known_host(&self.hostname, self.port, key_type, &key_base64) {
            KnownHostResult::Match => {
                tracing::debug!("Server key verified for {}", self.hostname);
                Ok(true)
            }
            KnownHostResult::Mismatch => {
                tracing::error!(
                    "HOST KEY MISMATCH for {}! The server key has changed. \
                     This could indicate a man-in-the-middle attack. \
                     Connection rejected.",
                    self.hostname
                );
                Ok(false)
            }
            KnownHostResult::NotFound => {
                tracing::info!(
                    "New host {} — adding {} key to known_hosts (TOFU)",
                    self.hostname,
                    key_type
                );
                known_hosts::add_known_host(&self.hostname, self.port, key_type, &key_base64);
                Ok(true)
            }
        }
    }

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!("SSH banner: {}", banner);
        self.send_event(SshEvent::Banner(banner.to_string()));
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_event(SshEvent::Data {
            channel,
            data: data.to_vec(),
        });
        Ok(())
    }

    async fn extended_data(
        &mut self,
        channel: ChannelId,
        ext: u32,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_event(SshEvent::ExtendedData {
            channel,
            data: data.to_vec(),
            ext,
        });
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_event(SshEvent::ChannelEof(channel));
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_event(SshEvent::ChannelClose(channel));
        Ok(())
    }

    async fn exit_status(
        &mut self,
        channel: ChannelId,
        exit_status: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send_event(SshEvent::ExitStatus {
            channel,
            code: exit_status,
        });
        Ok(())
    }

    async fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        let msg = match reason {
            DisconnectReason::ReceivedDisconnect(info) => {
                format!("Server disconnected: {:?}", info)
            }
            DisconnectReason::Error(e) => {
                format!("Connection error: {}", e)
            }
        };
        tracing::info!("{}", msg);
        self.send_event(SshEvent::Disconnected(msg));
        Ok(())
    }

    async fn channel_open_confirmation(
        &mut self,
        _id: ChannelId,
        _max_packet_size: u32,
        _window_size: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn channel_open_failure(
        &mut self,
        channel: ChannelId,
        reason: russh::ChannelOpenFailure,
        description: &str,
        _language: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::error!(
            "Channel {:?} open failed: {:?} - {}",
            channel,
            reason,
            description
        );
        self.send_event(SshEvent::Error(format!(
            "Channel open failed: {:?} - {}",
            reason, description
        )));
        Ok(())
    }

    async fn channel_success(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn channel_failure(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::warn!("Channel {:?} failure", channel);
        Ok(())
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        tracing::info!(
            "Server opened forwarded-tcpip channel: {}:{} (originator {}:{})",
            connected_address,
            connected_port,
            originator_address,
            originator_port,
        );

        let event = ForwardedTcpIpEvent {
            channel,
            connected_address: connected_address.to_string(),
            connected_port,
            originator_address: originator_address.to_string(),
            originator_port,
        };

        if self.forwarded_tcpip_tx.send(event).is_err() {
            tracing::error!(
                "Failed to send forwarded-tcpip event for {}:{} — receiver dropped",
                connected_address,
                connected_port,
            );
        }

        Ok(())
    }
}
