use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use shelldeck_ssh::client::SshClient;
use shelldeck_terminal::session::TerminalSession;
use uuid::Uuid;

use crate::terminal_view::SplitDirection;
use crate::toast::ToastLevel;

use super::Workspace;

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

        // Channel for SSH status feedback
        let (status_tx, status_rx) = std::sync::mpsc::channel::<Result<(), String>>();

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
                        let _ = status_tx
                            .send(Err(format!("Failed to create async runtime: {}", e)));
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let ssh_session = match client.connect(&conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            let msg = format!("SSH connection failed for {}: {}", conn.display_name(), e);
                            tracing::error!("{}", msg);
                            let _ = status_tx.send(Err(msg));
                            return;
                        }
                    };
                    tracing::info!("SSH connected to {}", conn.display_name());

                    let channel = match ssh_session.open_shell(rows as u32, cols as u32).await {
                        Ok(ch) => ch,
                        Err(e) => {
                            let msg = format!("Failed to open SSH shell for {}: {}", conn.display_name(), e);
                            tracing::error!("{}", msg);
                            let _ = status_tx.send(Err(msg));
                            return;
                        }
                    };
                    tracing::info!("SSH shell opened for {}", conn.display_name());

                    // Notify success
                    let _ = status_tx.send(Ok(()));

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
                        tracing::info!("SSH write loop ended");
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
                                        Some(SshChannelData::Data(data)) => {
                                            if data_tx.send(data).is_err() {
                                                break;
                                            }
                                        }
                                        Some(SshChannelData::Eof) | None => break,
                                    }
                                }
                            }
                        }
                        tracing::info!("SSH read loop ended");
                    });

                    tokio::select! {
                        _ = read_task => {}
                        _ = write_task => {}
                    }

                    tracing::info!("SSH session ended for {}", conn.display_name());
                });
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn SSH thread: {}", e);
            self.set_connection_status(
                conn_id,
                ConnectionStatus::Error(format!("Failed to start SSH thread: {}", e)),
                cx,
            );
            self.show_toast(
                format!("Failed to connect to {}: {}", title, e),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        // Spawn a GPUI task to listen for SSH status feedback
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            // Poll in a non-blocking way on the background executor
            let result = cx
                .background_executor()
                .spawn(async move { status_rx.recv().ok() })
                .await;

            if let Some(status) = result {
                let _ = weak.update(cx, |ws, cx| {
                    match status {
                        Ok(()) => {
                            ws.set_connection_status(conn_id, ConnectionStatus::Connected, cx);
                            ws.show_toast(
                                format!("Connected to {}", title),
                                ToastLevel::Success,
                                cx,
                            );
                        }
                        Err(msg) => {
                            ws.set_connection_status(
                                conn_id,
                                ConnectionStatus::Error(msg.clone()),
                                cx,
                            );
                            ws.show_toast(msg, ToastLevel::Error, cx);
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
        let conn_id = connection.id;

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
                                        Some(SshChannelData::Data(data)) => {
                                            if data_tx.send(data).is_err() {
                                                break;
                                            }
                                        }
                                        Some(SshChannelData::Eof) | None => break,
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
                format!("Failed to connect split to {}: {}", conn_id, e),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        self.show_toast(
            format!("Connecting split to {}", conn_id),
            ToastLevel::Info,
            cx,
        );
    }
}
