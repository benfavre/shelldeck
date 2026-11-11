use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use shelldeck_ssh::client::SshClient;
use shelldeck_ssh::session::SshSession;
use shelldeck_terminal::session::{SessionState, TerminalSession};
use std::sync::Arc;
use tokio::sync::mpsc;
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
    TmuxFailed(String),
    Ended(SshSessionEnd),
    /// `general.session_daemon`: the host's session daemon answered. The
    /// tab becomes a daemon pane; `panes` is what the daemon listed.
    #[cfg(unix)]
    DaemonReady(Vec<shelldeck_sessiond::protocol::PaneInfo>),
    /// `general.session_daemon`: this connection goes on without the
    /// daemon (tmux or a plain shell, as with the setting off). `ask`:
    /// offer to install the daemon on the host.
    #[cfg(unix)]
    DaemonFallback {
        reason: shelldeck_ssh::sessiond_install::FallbackReason,
        ask: bool,
    },
}

/// What the shell of one SSH tab needs, moved to the tab's thread.
struct ShellIo {
    attach_tmux: bool,
    rows: u16,
    cols: u16,
    workdir: Option<String>,
    data_tx: mpsc::UnboundedSender<Vec<u8>>,
    input_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    resize_rx: mpsc::UnboundedReceiver<(u16, u16)>,
    tmux_rx: mpsc::UnboundedReceiver<shelldeck_core::tmux::TmuxAction>,
}

/// Connect and authenticate. A failure is reported on `lifecycle_tx`.
async fn dial_ssh(
    conn: &Connection,
    lifecycle_tx: &mpsc::UnboundedSender<SshLifecycleEvent>,
) -> Option<Arc<SshSession>> {
    match SshClient::new().connect(conn).await {
        Ok(session) => {
            tracing::info!("SSH connected to {}", conn.display_name());
            Some(Arc::new(session))
        }
        Err(e) => {
            let msg = t!(
                "toast.ssh.connection_failed",
                name = conn.display_name(),
                error = e.to_string()
            )
            .to_string();
            tracing::error!("{}", msg);
            let _ = lifecycle_tx.send(SshLifecycleEvent::ConnectFailed(msg));
            None
        }
    }
}

/// The life of one SSH terminal on `ssh_session`: open the shell (tmux
/// automatic attachment when `io.attach_tmux`, else a plain PTY), relay
/// both directions and report how it ended.
async fn run_ssh_shell(
    ssh_session: Arc<SshSession>,
    conn: Connection,
    io: ShellIo,
    lifecycle_tx: mpsc::UnboundedSender<SshLifecycleEvent>,
) {
    let ShellIo {
        attach_tmux,
        rows,
        cols,
        workdir,
        data_tx,
        input_rx,
        resize_rx,
        mut tmux_rx,
    } = io;
    let opened = if attach_tmux {
        ssh_session
            .open_tmux_shell(rows as u32, cols as u32)
            .await
            .map(|(channel, client, initial)| (channel, Some(client), initial))
    } else {
        ssh_session
            .open_shell(rows as u32, cols as u32)
            .await
            .map(|channel| (channel, None, Vec::new()))
    };
    let (channel, tmux_client, initial_output) = match opened {
        Ok(ch) => ch,
        Err(e) => {
            let msg = t!(
                "toast.ssh.shell_failed",
                name = conn.display_name(),
                error = e.to_string()
            )
            .to_string();
            tracing::error!("{}", msg);
            let _ = lifecycle_tx.send(SshLifecycleEvent::ConnectFailed(msg));
            return;
        }
    };
    tracing::info!("SSH shell opened for {}", conn.display_name());

    let _ = lifecycle_tx.send(SshLifecycleEvent::Connected);

    if !initial_output.is_empty() {
        let _ = data_tx.send(initial_output);
    }

    let (mut channel_reader, channel_writer) = channel.split();

    if let Some(workdir) = workdir {
        let directory = if workdir == "~" {
            "\"$HOME\"".to_string()
        } else {
            shelldeck_core::util::shell_escape(&workdir)
        };
        let command = format!("cd -- {directory}\r");
        if channel_writer.write_all(command.as_bytes()).await.is_err() {
            let _ = lifecycle_tx.send(SshLifecycleEvent::Ended(
                SshSessionEnd::UnexpectedDisconnect,
            ));
            return;
        }
    }

    let mut input_rx = input_rx;
    let mut resize_rx = resize_rx;
    let write_task = tokio::spawn(async move {
        let mut resize_open = true;
        loop {
            tokio::select! {
                data = input_rx.recv() => {
                    let Some(data) = data else {
                        return SshSessionEnd::UserClosed;
                    };
                    if channel_writer.write_all(&data).await.is_err() {
                        return SshSessionEnd::UnexpectedDisconnect;
                    }
                }
                resize = resize_rx.recv(), if resize_open => {
                    if let Some((r, c)) = resize {
                        if let Err(e) = channel_writer.resize(r as u32, c as u32).await {
                            tracing::warn!("SSH resize failed: {}", e);
                        }
                    } else {
                        resize_open = false;
                    }
                }
            }
        }
    });

    let read_task = tokio::spawn(async move {
        use shelldeck_ssh::session::SshChannelData;
        loop {
            match channel_reader.read().await {
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
    });

    let control_events = lifecycle_tx.clone();
    let controls = async move {
        if let Some(client) = tmux_client {
            while let Some(action) = tmux_rx.recv().await {
                if let Err(error) = client.apply(&*ssh_session, action).await {
                    let _ = control_events.send(SshLifecycleEvent::TmuxFailed(error.to_string()));
                }
            }
        } else {
            // Keeps the connection alive for the life of the shell.
            let _session = ssh_session;
            std::future::pending::<()>().await;
        }
    };
    let end = tokio::select! {
        // Closing a tab makes both sides unwind; prefer the
        // explicit input-channel close over a secondary read
        // failure so voluntary closes never notify.
        biased;
        result = write_task => result.unwrap_or(SshSessionEnd::UnexpectedDisconnect),
        result = read_task => result.unwrap_or(SshSessionEnd::UnexpectedDisconnect),
        _ = controls => SshSessionEnd::UserClosed,
    };

    tracing::info!(
        end = ?end,
        "SSH session ended for {}",
        conn.display_name()
    );
    let _ = lifecycle_tx.send(SshLifecycleEvent::Ended(end));
}

/// A connection of its own for one SSH terminal, on a runtime of this
/// thread: the behaviour with `general.session_daemon` off.
fn run_ssh_shell_on_own_connection(
    conn: Connection,
    io: ShellIo,
    lifecycle_tx: mpsc::UnboundedSender<SshLifecycleEvent>,
) {
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
        let Some(ssh_session) = dial_ssh(&conn, &lifecycle_tx).await else {
            return;
        };
        run_ssh_shell(ssh_session, conn, io, lifecycle_tx).await;
    });
}

/// `general.session_daemon` on: reach the host's session daemon first
/// (installing it when the user agreed for this host). When it answers,
/// the tab is handed over to it; otherwise this thread goes on exactly as
/// with the setting off, on the connection already made when there is one.
#[cfg(unix)]
fn run_ssh_daemon_or_shell(
    host: shelldeck_ssh::sessiond_host::RemoteHost,
    consent: shelldeck_ssh::sessiond_install::InstallConsent,
    conn: Connection,
    io: ShellIo,
    lifecycle_tx: mpsc::UnboundedSender<SshLifecycleEvent>,
) {
    use shelldeck_ssh::sessiond_host::Prepared;
    match host.prepare(consent) {
        Prepared::Ready { panes } => {
            tracing::info!("Session daemon ready on {}", conn.display_name());
            let _ = lifecycle_tx.send(SshLifecycleEvent::DaemonReady(panes));
        }
        Prepared::ConnectFailed(e) => {
            let msg = t!(
                "toast.ssh.connection_failed",
                name = conn.display_name(),
                error = e.to_string()
            )
            .to_string();
            tracing::error!("{}", msg);
            let _ = lifecycle_tx.send(SshLifecycleEvent::ConnectFailed(msg));
        }
        Prepared::Fallback {
            reason,
            ask,
            session,
        } => {
            tracing::info!(
                ?reason,
                "No session daemon for {}, using the configured SSH terminal",
                conn.display_name()
            );
            let _ = lifecycle_tx.send(SshLifecycleEvent::DaemonFallback { reason, ask });
            match (session, super::remote_daemon::runtime()) {
                // The connection was made on the shared runtime: the
                // shell's tasks run there, this thread only waits.
                (Some(session), Ok(runtime)) => {
                    runtime
                        .handle()
                        .block_on(run_ssh_shell(session, conn, io, lifecycle_tx))
                }
                _ => run_ssh_shell_on_own_connection(conn, io, lifecycle_tx),
            }
        }
    }
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
        self.connect_ssh_at(connection, None, cx);
    }

    pub(super) fn connect_ssh_at(
        &mut self,
        connection: Connection,
        workdir: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.enter_dev_mode(cx) {
            return;
        }
        let title = connection.display_name().to_string();
        let conn_id = connection.id;

        let (rows, cols) = self.terminal.read(cx).grid_size();
        let attach_tmux = workdir.is_none() && self.app_config.general.auto_attach_tmux;

        // `general.session_daemon`: the tab is a pane of the host's
        // session daemon when the host can run it. A tab opened in a
        // given directory keeps the plain path, like the tmux attachment.
        #[cfg(unix)]
        let daemon_host = if workdir.is_none() && self.app_config.general.session_daemon {
            match super::remote_daemon::host_for(&connection) {
                Ok(host) => Some(host),
                Err(error) => {
                    tracing::warn!("Session daemon not used for {title}: {error}");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(unix)]
        if let Some(host) = daemon_host.as_ref().filter(|host| host.is_ready()) {
            // The host's daemon is connected already: a new pane on the
            // shared connection, no second SSH connection.
            if self.open_remote_daemon_tab(host, conn_id, &title, cx) {
                return;
            }
        }
        #[cfg(unix)]
        let daemon_host = daemon_host.map(|host| {
            self.watch_remote_daemon_hosts(cx);
            super::remote_daemon::begin_connect(conn_id);
            let consent = super::remote_daemon::consent(&self.app_config.general, conn_id);
            (host, consent)
        });

        let (mut session, data_tx, input_rx) =
            match TerminalSession::spawn_ssh(title.clone(), rows, cols) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to create SSH session: {}", e);
                    #[cfg(unix)]
                    if daemon_host.is_some() {
                        super::remote_daemon::end_connect(conn_id);
                    }
                    return;
                }
            };
        let session_id = session.id;

        let (tmux_tx, tmux_rx) = tokio::sync::mpsc::unbounded_channel();
        if attach_tmux {
            session.set_tmux_control(tmux_tx);
        }

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
        #[cfg(unix)]
        let daemon_connect = daemon_host.is_some();
        let io = ShellIo {
            attach_tmux,
            rows,
            cols,
            workdir,
            data_tx,
            input_rx,
            resize_rx,
            tmux_rx,
        };
        let spawn_result = std::thread::Builder::new()
            .name(format!("ssh-{}", title))
            .spawn(move || {
                #[cfg(unix)]
                if let Some((host, consent)) = daemon_host {
                    run_ssh_daemon_or_shell(host, consent, conn, io, lifecycle_tx);
                    return;
                }
                run_ssh_shell_on_own_connection(conn, io, lifecycle_tx);
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn SSH thread: {}", e);
            #[cfg(unix)]
            if daemon_connect {
                super::remote_daemon::end_connect(conn_id);
            }
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
            // This tab still counts as "connecting to the host's daemon".
            #[cfg(unix)]
            let mut daemon_connecting = daemon_connect;
            while let Some(event) = lifecycle_rx.recv().await {
                #[cfg(unix)]
                if matches!(event, SshLifecycleEvent::DaemonReady(_)) {
                    daemon_connecting = false;
                }
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
                        #[cfg(unix)]
                        SshLifecycleEvent::DaemonReady(panes) => {
                            ws.remote_daemon_ready(conn_id, session_id, &title, panes, cx);
                        }
                        #[cfg(unix)]
                        SshLifecycleEvent::DaemonFallback { reason, ask } => {
                            // The connection goes on: `Connected` or
                            // `ConnectFailed` follows.
                            daemon_connecting = false;
                            super::remote_daemon::end_connect(conn_id);
                            ws.remote_daemon_fallback(conn_id, &title, reason, ask, cx);
                        }
                        SshLifecycleEvent::ConnectFailed(msg) => {
                            #[cfg(unix)]
                            if std::mem::take(&mut daemon_connecting) {
                                super::remote_daemon::end_connect(conn_id);
                            }
                            ws.set_connection_status(
                                conn_id,
                                ConnectionStatus::Error(msg.clone()),
                                cx,
                            );
                            ws.show_toast(msg, ToastLevel::Error, cx);
                        }
                        SshLifecycleEvent::TmuxFailed(error) => {
                            ws.show_toast(
                                t!("toast.ssh.tmux_control_failed", error = error).to_string(),
                                ToastLevel::Error,
                                cx,
                            );
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
        if !self.enter_dev_mode(cx) {
            return;
        }
        let title = format!("{} (split)", connection.display_name());
        let _conn_id = connection.id;

        // A split of a session daemon tab is another pane of the same
        // daemon, on the host's shared SSH connection.
        #[cfg(unix)]
        if self.split_remote_daemon_pane(&connection, direction, cx) {
            return;
        }

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

                    let (mut channel_reader, channel_writer) = channel.split();

                    let mut input_rx = input_rx;
                    let mut resize_rx = resize_rx;
                    let write_task = tokio::spawn(async move {
                        let mut resize_open = true;
                        // Auto-attach (or create) a tmux session at session start
                        // when enabled. Runs exactly once before the input loop.
                        if attach_tmux {
                            let _ = channel_writer
                                .write_all(b"tmux new-session -A -s main\n")
                                .await;
                        }
                        loop {
                            tokio::select! {
                                data = input_rx.recv() => {
                                    let Some(data) = data else {
                                        break;
                                    };
                                    if channel_writer.write_all(&data).await.is_err() {
                                        break;
                                    }
                                }
                                resize = resize_rx.recv(), if resize_open => {
                                    if let Some((r, c)) = resize {
                                        if let Err(e) = channel_writer.resize(r as u32, c as u32).await {
                                            tracing::warn!("SSH split resize failed: {}", e);
                                        }
                                    } else {
                                        resize_open = false;
                                    }
                                }
                            }
                        }
                        tracing::info!("SSH split write loop ended");
                    });

                    let read_task = tokio::spawn(async move {
                        use shelldeck_ssh::session::SshChannelData;
                        // CleanEnd and ConnectionLost both end the loop, so the
                        // `Data` arm is the only one that continues it.
                        while let SshChannelData::Data(data) = channel_reader.read().await {
                            if data_tx.send(data).is_err() {
                                break;
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

/// The user's answer to the install question of one host.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallAnswer {
    Install,
    Later,
    Never,
}

/// `general.session_daemon` on SSH hosts: what the workspace does around
/// the host objects of `remote_daemon.rs`. Nothing here waits for a host.
#[cfg(unix)]
impl Workspace {
    /// Start the one loop that applies host events (idle, lost,
    /// reconnected) to the connection status. No-op once it runs.
    fn watch_remote_daemon_hosts(&mut self, cx: &mut Context<Self>) {
        let Some(mut events) = super::remote_daemon::take_events() else {
            return;
        };
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            while let Some((conn_id, event)) = events.recv().await {
                let _ = weak.update(cx, |ws, cx| {
                    ws.remote_daemon_host_event(conn_id, event, cx);
                });
            }
        })
        .detach();
    }

    /// Whether a tab of this connection still has something behind it.
    fn connection_has_live_tab(&self, conn_id: Uuid, cx: &Context<Self>) -> bool {
        use shelldeck_terminal::attached::AttachedPhase;
        let terminal = self.terminal.read(cx);
        terminal
            .tabs
            .iter()
            .zip(terminal.pane.sessions.iter())
            .any(|(tab, session)| {
                tab.connection_id == Some(conn_id)
                    && match session.attached_status() {
                        Some(status) => matches!(
                            status.phase(),
                            AttachedPhase::Connecting
                                | AttachedPhase::Live
                                | AttachedPhase::Reconnecting
                        ),
                        None => tab.state == SessionState::Running,
                    }
            })
    }

    fn remote_daemon_host_event(
        &mut self,
        conn_id: Uuid,
        event: shelldeck_ssh::sessiond_host::HostEvent,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_ssh::sessiond_host::HostEvent;
        let Some(name) = self
            .connections
            .iter()
            .find(|conn| conn.id == conn_id)
            .map(|conn| conn.display_name().to_string())
        else {
            return;
        };
        match event {
            // The last daemon pane of the host was closed or ended: the
            // shared SSH connection is gone.
            HostEvent::Idle => {
                if !self.connection_has_live_tab(conn_id, cx) {
                    self.set_connection_status(conn_id, ConnectionStatus::Disconnected, cx);
                }
            }
            HostEvent::ConnectionLost => {
                // The panes keep running on the host and the tabs
                // reattach by themselves.
                self.set_connection_status(conn_id, ConnectionStatus::Connecting, cx);
                self.show_toast(
                    t!("toast.ssh.connection_lost", name = name.as_str()).to_string(),
                    ToastLevel::Warning,
                    cx,
                );
                if self.app_config.tray.notify_ssh_disconnect {
                    self.emit_tray_notification(TrayNotification::SshDisconnected { name });
                }
            }
            HostEvent::Reconnected => {
                self.set_connection_status(conn_id, ConnectionStatus::Connected, cx);
            }
        }
        cx.notify();
    }

    /// A new tab on a host whose daemon is connected already. `false`:
    /// the session could not be created, use the ordinary path.
    fn open_remote_daemon_tab(
        &mut self,
        host: &shelldeck_ssh::sessiond_host::RemoteHost,
        conn_id: Uuid,
        title: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let (rows, cols) = self.terminal.read(cx).grid_size();
        let scrollback = self.terminal.read(cx).configured_scrollback();
        let session = match super::remote_daemon::open_new(host, title, rows, cols, scrollback) {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!("Session daemon pane not opened for {title}: {error}");
                return false;
            }
        };
        self.watch_remote_daemon_hosts(cx);
        self.terminal.update(cx, |terminal, cx| {
            terminal.add_session_with_connection(session, Some(conn_id));
            terminal.ensure_refresh_running(cx);
            cx.notify();
        });
        self.sync_terminal_tab_count(cx);
        self.set_connection_status(conn_id, ConnectionStatus::Connected, cx);
        true
    }

    /// A split of a session daemon tab: another pane of the same daemon.
    /// `false`: not a daemon tab, use the ordinary path.
    fn split_remote_daemon_pane(
        &mut self,
        connection: &Connection,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) -> bool {
        let on_daemon_pane = self
            .terminal
            .read(cx)
            .active_session()
            .is_some_and(|session| session.is_daemon_attached());
        if !on_daemon_pane || !self.app_config.general.session_daemon {
            return false;
        }
        let Some(host) = super::remote_daemon::existing_host(connection.id) else {
            return false;
        };
        let (rows, cols) = self.terminal.read(cx).grid_size();
        let scrollback = self.terminal.read(cx).configured_scrollback();
        let title = connection.display_name().to_string();
        match super::remote_daemon::open_new(&host, &title, rows, cols, scrollback) {
            Ok(session) => {
                self.terminal.update(cx, |terminal, cx| {
                    terminal.set_split_session(session, direction, cx);
                });
                true
            }
            Err(error) => {
                tracing::warn!("Session daemon split not opened for {title}: {error}");
                false
            }
        }
    }

    /// The host's daemon answered for the tab that shows `session_id`:
    /// the tab becomes a daemon pane (one left by an earlier run when
    /// there is one, else a new one), and the other panes left on the
    /// host come back as tabs.
    fn remote_daemon_ready(
        &mut self,
        conn_id: Uuid,
        session_id: Uuid,
        title: &str,
        panes: Vec<shelldeck_sessiond::protocol::PaneInfo>,
        cx: &mut Context<Self>,
    ) {
        let others_connecting = super::remote_daemon::end_connect(conn_id) > 0;
        let Some(host) = super::remote_daemon::existing_host(conn_id) else {
            return;
        };
        // Never on this thread: releasing waits for the host.
        let release = |host: shelldeck_ssh::sessiond_host::RemoteHost| {
            let _ = std::thread::Builder::new()
                .name("sessiond-ssh-release".into())
                .spawn(move || host.release_if_idle());
        };
        if !self.terminal.read(cx).has_session(session_id) {
            // The tab was closed while it was connecting.
            release(host);
            return;
        }
        let plan = super::remote_daemon::ready_plan(
            &panes,
            &host.supervisor().open_panes(),
            others_connecting,
        );
        let (rows, cols) = self.terminal.read(cx).grid_size();
        let scrollback = self.terminal.read(cx).configured_scrollback();
        let opened = match plan.tab {
            Some(pane) => {
                super::remote_daemon::open_existing(&host, pane, title, rows, cols, scrollback)
            }
            None => super::remote_daemon::open_new(&host, title, rows, cols, scrollback),
        };
        let session = match opened {
            Ok(session) => session,
            Err(error) => {
                let msg = t!(
                    "toast.ssh.session_daemon.open_failed",
                    name = title,
                    error = error
                )
                .to_string();
                self.set_connection_status(conn_id, ConnectionStatus::Error(msg.clone()), cx);
                self.show_toast(msg, ToastLevel::Error, cx);
                release(host);
                return;
            }
        };
        let mut restored = usize::from(plan.tab.is_some());
        self.terminal.update(cx, |terminal, cx| {
            terminal.replace_session(session_id, session);
            for pane in &plan.extra_tabs {
                match super::remote_daemon::open_existing(
                    &host, *pane, title, rows, cols, scrollback,
                ) {
                    Ok(session) => {
                        terminal.add_session_with_connection(session, Some(conn_id));
                        restored += 1;
                    }
                    Err(error) => tracing::warn!(%pane, "remote pane not offered back: {error}"),
                }
            }
            if !plan.extra_tabs.is_empty() {
                // Keep the tab the user just opened in front.
                terminal.select_tab(session_id);
            }
            terminal.ensure_refresh_running(cx);
            cx.notify();
        });
        self.sync_terminal_tab_count(cx);
        self.set_connection_status(conn_id, ConnectionStatus::Connected, cx);
        let message = if restored > 0 {
            t!(
                "toast.ssh.session_daemon.restored",
                name = title,
                count = restored
            )
        } else {
            t!("toast.ssh.connected", name = title)
        };
        self.show_toast(message.to_string(), ToastLevel::Success, cx);
    }

    /// This connection goes on without the daemon. One notice per host
    /// and per run; when the daemon could be installed and the user was
    /// never asked, the notice is the question.
    fn remote_daemon_fallback(
        &mut self,
        conn_id: Uuid,
        title: &str,
        reason: shelldeck_ssh::sessiond_install::FallbackReason,
        ask: bool,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_ssh::sessiond_install::{fallback_backend, FallbackBackend};
        let backend = match fallback_backend(self.app_config.general.auto_attach_tmux) {
            FallbackBackend::Tmux => t!("toast.ssh.session_daemon.backend.tmux"),
            FallbackBackend::PlainShell => t!("toast.ssh.session_daemon.backend.plain"),
        };
        if ask && super::remote_daemon::first_ask(conn_id) {
            super::remote_daemon::first_notice(conn_id);
            let weak = cx.entity().downgrade();
            let answer = |label: std::borrow::Cow<'_, str>, answer: InstallAnswer| {
                let weak = weak.clone();
                crate::toast::ToastAction::new(label.to_string(), move |cx| {
                    let _ = weak.update(cx, |ws, cx| {
                        ws.answer_remote_daemon_install(conn_id, answer, cx);
                    });
                })
            };
            let actions = vec![
                answer(
                    t!("toast.ssh.session_daemon.install_prompt.install"),
                    InstallAnswer::Install,
                ),
                answer(
                    t!("toast.ssh.session_daemon.install_prompt.later"),
                    InstallAnswer::Later,
                ),
                answer(
                    t!("toast.ssh.session_daemon.install_prompt.never"),
                    InstallAnswer::Never,
                ),
            ];
            let message = t!(
                "toast.ssh.session_daemon.install_prompt.message",
                name = title,
                backend = backend
            )
            .to_string();
            // A question, not a notification: shown whatever the
            // notification setting says, and it never blocks the app.
            self.toasts.update(cx, |toasts, cx| {
                toasts.push_prompt(message, actions, cx);
            });
            return;
        }
        if super::remote_daemon::first_notice(conn_id) {
            self.show_toast(
                t!(
                    "toast.ssh.session_daemon.fallback",
                    name = title,
                    reason = super::remote_daemon::fallback_reason_text(reason),
                    backend = backend
                )
                .to_string(),
                ToastLevel::Info,
                cx,
            );
        }
    }

    /// Record the user's answer for a host. "Install" also uploads the
    /// daemon right away, in the background, so the next terminal of that
    /// host uses it.
    fn answer_remote_daemon_install(
        &mut self,
        conn_id: Uuid,
        answer: InstallAnswer,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_core::config::app_config::RemoteDaemonInstall;
        let recorded = match answer {
            InstallAnswer::Install => RemoteDaemonInstall::Allow,
            InstallAnswer::Never => RemoteDaemonInstall::Never,
            // Undecided: asked again in a later run.
            InstallAnswer::Later => return,
        };
        self.app_config
            .general
            .set_remote_daemon_install(conn_id, recorded);
        if let Err(error) = self.app_config.save() {
            tracing::error!("Failed to save the session daemon answer: {error}");
        }
        self.sync_settings_config(cx);
        let name = self
            .connections
            .iter()
            .find(|conn| conn.id == conn_id)
            .map(|conn| conn.display_name().to_string())
            .unwrap_or_default();
        if answer == InstallAnswer::Never {
            self.show_toast(
                t!(
                    "toast.ssh.session_daemon.never_recorded",
                    name = name.as_str()
                )
                .to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        let Some(host) = super::remote_daemon::existing_host(conn_id) else {
            return;
        };
        self.show_toast(
            t!("toast.ssh.session_daemon.installing", name = name.as_str()).to_string(),
            ToastLevel::Info,
            cx,
        );
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let installed = cx
                .background_executor()
                .spawn(async move { host.install() })
                .await;
            let _ = this.update(cx, |ws, cx| match installed {
                Ok(()) => ws.show_toast(
                    t!("toast.ssh.session_daemon.installed", name = name.as_str()).to_string(),
                    ToastLevel::Success,
                    cx,
                ),
                Err(error) => ws.show_toast(
                    t!(
                        "toast.ssh.session_daemon.install_failed",
                        name = name.as_str(),
                        error = error.to_string()
                    )
                    .to_string(),
                    ToastLevel::Error,
                    cx,
                ),
            });
        })
        .detach();
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
