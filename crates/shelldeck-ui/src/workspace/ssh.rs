use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use shelldeck_ssh::client::SshClient;
use shelldeck_terminal::session::{SessionState, TerminalSession};
use uuid::Uuid;

use crate::t;
use crate::terminal_view::SplitDirection;
use crate::toast::ToastLevel;

use super::{TrayNotification, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SshSessionEnd {
    UserClosed,
    CleanRemoteExit,
    UnexpectedDisconnect,
}

#[derive(Debug)]
enum SshLifecycleEvent {
    Connected,
    ConnectFailed(String),
    Ended(SshSessionEnd),
}

fn disconnect_notification(end: SshSessionEnd, name: &str) -> Option<TrayNotification> {
    matches!(end, SshSessionEnd::UnexpectedDisconnect).then(|| TrayNotification::SshDisconnected {
        name: name.to_string(),
    })
}

impl Workspace {
    /// Update a connection's status and refresh sidebar.
    pub(super) fn set_connection_status(
        &mut self,
        conn_id: Uuid,
        status: ConnectionStatus,
        cx: &mut Context<Self>,
    ) {
        if let Some(conn) = self.connections.iter_mut().find(|c| c.id == conn_id) {
            conn.status = status;
        }
        let conns = self.connections.clone();
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_connections(conns.clone());
            cx.notify();
        });
        self.server_sync.update(cx, |view, cx| {
            view.set_connections(conns.clone(), cx);
        });
        self.sites.update(cx, |view, _| {
            view.set_connections(conns);
        });
        self.update_dashboard_stats(cx);
        self.publish_tray_state(cx);
    }

    /// Initiate an SSH connection to `connection`.
    pub(super) fn connect_ssh(&mut self, connection: Connection, cx: &mut Context<Self>) {
        let title = connection.display_name().to_string();
        let conn_id = connection.id;

        let (rows, cols) = self.terminal.read(cx).grid_size();
        let attach_tmux = self.app_config.general.auto_attach_tmux;

        let (mut session, data_tx, input_rx) =
            match TerminalSession::spawn_ssh(title.clone(), rows, cols) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to create SSH session: {}", e);
                    return;
                }
            };
        let session_id = session.id;

        let (resize_tx, resize_rx) = tokio::sync::mpsc::unbounded_channel::<(u16, u16)>();
        session.set_resize_fn(Box::new(move |rows, cols| {
            let _ = resize_tx.send((rows, cols));
        }));

        self.terminal.update(cx, |terminal, cx| {
            terminal.add_session_with_connection(session, Some(conn_id));
            terminal.ensure_refresh_running(cx);
            cx.notify();
        });
        self.sync_terminal_tab_count(cx);

        // Mark as connecting
        self.set_connection_status(conn_id, ConnectionStatus::Connecting, cx);

        // Channel for the complete SSH lifecycle. It crosses the dedicated
        // runtime thread without polling and lets the Workspace distinguish a
        // closed tab, a clean shell exit, and a lost transport.
        let (lifecycle_tx, mut lifecycle_rx) =
            tokio::sync::mpsc::unbounded_channel::<SshLifecycleEvent>();

        let conn = connection;
        let spawn_result = std::thread::Builder::new()
            .name(format!("ssh-{}", title))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = lifecycle_tx.send(SshLifecycleEvent::ConnectFailed(
                            t!("toast.ssh.runtime_failed", error = e.to_string()).to_string(),
                        ));
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let ssh_session = match client.connect(&conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            let msg = t!(
                                "toast.ssh.connection_failed",
                                name = conn.display_name(),
                                error = e.to_string()
                            )
                            .to_string();
                            tracing::error!("{}", msg);
                            let _ =
                                lifecycle_tx.send(SshLifecycleEvent::ConnectFailed(msg));
                            return;
                        }
                    };
                    tracing::info!("SSH connected to {}", conn.display_name());

                    let channel = match ssh_session.open_shell(rows as u32, cols as u32).await {
                        Ok(ch) => ch,
                        Err(e) => {
                            let msg = t!(
                                "toast.ssh.shell_failed",
                                name = conn.display_name(),
                                error = e.to_string()
                            )
                            .to_string();
                            tracing::error!("{}", msg);
                            let _ =
                                lifecycle_tx.send(SshLifecycleEvent::ConnectFailed(msg));
                            return;
                        }
                    };
                    tracing::info!("SSH shell opened for {}", conn.display_name());

                    let _ = lifecycle_tx.send(SshLifecycleEvent::Connected);

                    let (mut channel_reader, mut channel_writer) = channel.split();

                    let mut input_rx = input_rx;
                    let write_task = tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        // Auto-attach (or create) a tmux session at session start
                        // when enabled. Runs exactly once before the input loop.
                        if attach_tmux
                            && channel_writer
                                .write_all(b"tmux new-session -A -s main\n")
                                .await
                                .is_err()
                        {
                            return SshSessionEnd::UnexpectedDisconnect;
                        }
                        loop {
                            match input_rx.recv().await {
                                Some(data) if channel_writer.write_all(&data).await.is_err() => {
                                    return SshSessionEnd::UnexpectedDisconnect;
                                }
                                Some(_) => {}
                                None => return SshSessionEnd::UserClosed,
                            }
                        }
                    });

                    let mut resize_rx = resize_rx;
                    let read_task = tokio::spawn(async move {
                        use shelldeck_ssh::session::SshChannelData;
                        loop {
                            tokio::select! {
                                biased;
                                Some((r, c)) = resize_rx.recv() => {
                                    if let Err(e) = channel_reader.resize(r as u32, c as u32).await {
                                        tracing::warn!("SSH resize failed: {}", e);
                                    }
                                }
                                msg = channel_reader.read() => {
                                    match msg {
                                        SshChannelData::Data(data) => {
                                            if data_tx.send(data).is_err() {
                                                return SshSessionEnd::UserClosed;
                                            }
                                        }
                                        SshChannelData::CleanEnd => {
                                            return SshSessionEnd::CleanRemoteExit;
                                        }
                                        SshChannelData::ConnectionLost => {
                                            return SshSessionEnd::UnexpectedDisconnect;
                                        }
                                    }
                                }
                            }
                        }
                    });

                    let end = tokio::select! {
                        // Closing a tab makes both sides unwind; prefer the
                        // explicit input-channel close over a secondary read
                        // failure so voluntary closes never notify.
                        biased;
                        result = write_task => result.unwrap_or(SshSessionEnd::UnexpectedDisconnect),
                        result = read_task => result.unwrap_or(SshSessionEnd::UnexpectedDisconnect),
                    };

                    tracing::info!(
                        end = ?end,
                        "SSH session ended for {}",
                        conn.display_name()
                    );
                    let _ = lifecycle_tx.send(SshLifecycleEvent::Ended(end));
                });
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn SSH thread: {}", e);
            self.set_connection_status(
                conn_id,
                ConnectionStatus::Error(
                    t!("toast.ssh.thread_start_failed", error = e.to_string()).to_string(),
                ),
                cx,
            );
            self.show_toast(
                t!(
                    "toast.ssh.connect_failed",
                    name = title.as_str(),
                    error = e.to_string()
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        // Apply every lifecycle event on GPUI's foreground executor.
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            while let Some(event) = lifecycle_rx.recv().await {
                let _ = weak.update(cx, |ws, cx| {
                    match event {
                        SshLifecycleEvent::Connected => {
                            ws.set_connection_status(conn_id, ConnectionStatus::Connected, cx);
                            ws.show_toast(
                                t!("toast.ssh.connected", name = title.as_str()).to_string(),
                                ToastLevel::Success,
                                cx,
                            );
                        }
                        SshLifecycleEvent::ConnectFailed(msg) => {
                            ws.set_connection_status(
                                conn_id,
                                ConnectionStatus::Error(msg.clone()),
                                cx,
                            );
                            ws.show_toast(msg, ToastLevel::Error, cx);
                        }
                        SshLifecycleEvent::Ended(end) => {
                            let connection_lost =
                                t!("toast.ssh.connection_lost", name = title.as_str()).to_string();
                            let session_state = match end {
                                SshSessionEnd::UserClosed => None,
                                SshSessionEnd::CleanRemoteExit => Some(SessionState::Exited(0)),
                                SshSessionEnd::UnexpectedDisconnect => {
                                    Some(SessionState::Error(connection_lost.clone()))
                                }
                            };
                            if let Some(session_state) = session_state {
                                ws.terminal.update(cx, |terminal, cx| {
                                    if let Some(tab) =
                                        terminal.tabs.iter_mut().find(|tab| tab.id == session_id)
                                    {
                                        tab.state = session_state.clone();
                                    }
                                    if let Some(session) = terminal
                                        .pane
                                        .sessions
                                        .iter_mut()
                                        .find(|session| session.id == session_id)
                                    {
                                        session.state = session_state;
                                    }
                                    cx.notify();
                                });
                            }
                            let has_other_session = ws.terminal.read(cx).tabs.iter().any(|tab| {
                                tab.id != session_id
                                    && tab.connection_id == Some(conn_id)
                                    && tab.state == SessionState::Running
                            });
                            if !has_other_session {
                                let status = if end == SshSessionEnd::UnexpectedDisconnect {
                                    ConnectionStatus::Error(connection_lost)
                                } else {
                                    ConnectionStatus::Disconnected
                                };
                                ws.set_connection_status(conn_id, status, cx);
                            }
                            if ws.app_config.tray.notify_ssh_disconnect {
                                if let Some(notification) =
                                    disconnect_notification(end, title.as_str())
                                {
                                    ws.emit_tray_notification(notification);
                                }
                            }
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Initiate an SSH connection for a split pane on the current tab.
    pub(super) fn connect_ssh_split(
        &mut self,
        connection: Connection,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) {
        let title = format!("{} (split)", connection.display_name());
        let _conn_id = connection.id;

        let (rows, cols) = self.terminal.read(cx).grid_size();
        let attach_tmux = self.app_config.general.auto_attach_tmux;

        let (mut session, data_tx, input_rx) =
            match TerminalSession::spawn_ssh(title.clone(), rows, cols) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to create SSH split session: {}", e);
                    return;
                }
            };

        let (resize_tx, resize_rx) = tokio::sync::mpsc::unbounded_channel::<(u16, u16)>();
        session.set_resize_fn(Box::new(move |rows, cols| {
            let _ = resize_tx.send((rows, cols));
        }));

        // Inject the session into the terminal view's split
        let terminal = self.terminal.clone();
        terminal.update(cx, |terminal, cx| {
            terminal.set_split_session(session, direction, cx);
        });

        let conn = connection;
        let spawn_result = std::thread::Builder::new()
            .name(format!("ssh-split-{}", title))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("Failed to create async runtime for SSH split: {}", e);
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let ssh_session = match client.connect(&conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!("SSH split connection failed for {}: {}", conn.display_name(), e);
                            return;
                        }
                    };
                    tracing::info!("SSH split connected to {}", conn.display_name());

                    let channel = match ssh_session.open_shell(rows as u32, cols as u32).await {
                        Ok(ch) => ch,
                        Err(e) => {
                            tracing::error!("Failed to open SSH split shell for {}: {}", conn.display_name(), e);
                            return;
                        }
                    };
                    tracing::info!("SSH split shell opened for {}", conn.display_name());

                    let (mut channel_reader, mut channel_writer) = channel.split();

                    let mut input_rx = input_rx;
                    let write_task = tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        // Auto-attach (or create) a tmux session at session start
                        // when enabled. Runs exactly once before the input loop.
                        if attach_tmux {
                            let _ = channel_writer
                                .write_all(b"tmux new-session -A -s main\n")
                                .await;
                        }
                        while let Some(data) = input_rx.recv().await {
                            if channel_writer.write_all(&data).await.is_err() {
                                break;
                            }
                        }
                        tracing::info!("SSH split write loop ended");
                    });

                    let mut resize_rx = resize_rx;
                    let read_task = tokio::spawn(async move {
                        use shelldeck_ssh::session::SshChannelData;
                        loop {
                            tokio::select! {
                                biased;
                                Some((r, c)) = resize_rx.recv() => {
                                    if let Err(e) = channel_reader.resize(r as u32, c as u32).await {
                                        tracing::warn!("SSH split resize failed: {}", e);
                                    }
                                }
                                msg = channel_reader.read() => {
                                    match msg {
                                        SshChannelData::Data(data) => {
                                            if data_tx.send(data).is_err() {
                                                break;
                                            }
                                        }
                                        SshChannelData::CleanEnd
                                        | SshChannelData::ConnectionLost => break,
                                    }
                                }
                            }
                        }
                        tracing::info!("SSH split read loop ended");
                    });

                    tokio::select! {
                        _ = read_task => {}
                        _ = write_task => {}
                    }

                    tracing::info!("SSH split session ended for {}", conn.display_name());
                });
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn SSH split thread: {}", e);
            self.show_toast(
                t!(
                    "toast.ssh.split_connect_failed",
                    name = title.as_str(),
                    error = e.to_string()
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        self.show_toast(
            t!("toast.ssh.split_connecting", name = title.as_str()).to_string(),
            ToastLevel::Info,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{disconnect_notification, SshSessionEnd};
    use crate::workspace::TrayNotification;

    // SDTEST-1412
    #[test]
    fn only_unexpected_ssh_transport_loss_notifies_with_exact_identity() {
        assert_eq!(
            disconnect_notification(SshSessionEnd::UserClosed, "production"),
            None
        );
        assert_eq!(
            disconnect_notification(SshSessionEnd::CleanRemoteExit, "production"),
            None
        );
        assert_eq!(
            disconnect_notification(SshSessionEnd::UnexpectedDisconnect, "production"),
            Some(TrayNotification::SshDisconnected {
                name: "production".to_string(),
            })
        );
    }
}
