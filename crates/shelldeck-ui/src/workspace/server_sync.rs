use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::Connection;
use shelldeck_core::models::server_sync::SyncProfile;
use shelldeck_ssh::client::SshClient;
use uuid::Uuid;

use crate::server_sync_view::{PanelSide, ServerSyncEvent, LOCAL_MACHINE_ID};
use crate::t;
use crate::toast::ToastLevel;

use super::{ActiveScript, Workspace};

impl Workspace {
    pub(super) fn handle_server_sync_event(
        &mut self,
        event: &ServerSyncEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            ServerSyncEvent::ListFiles {
                connection_id,
                path,
                panel,
            } => {
                let conn_id = *connection_id;
                let path = path.clone();
                let panel = *panel;

                if conn_id == LOCAL_MACHINE_ID {
                    self.list_local_files(path, panel, cx);
                } else if let Some(conn) =
                    self.connections.iter().find(|c| c.id == conn_id).cloned()
                {
                    self.list_remote_files(conn, path, panel, cx);
                }
            }
            ServerSyncEvent::DiscoverServices {
                connection_id,
                panel,
            } => {
                let conn_id = *connection_id;
                let panel = *panel;

                if conn_id == LOCAL_MACHINE_ID {
                    self.discover_local_services(panel, cx);
                } else if let Some(conn) =
                    self.connections.iter().find(|c| c.id == conn_id).cloned()
                {
                    self.discover_remote_services(conn, panel, cx);
                }
            }
            ServerSyncEvent::StartSync(profile) => {
                self.start_sync_operation(profile.clone(), cx);
            }
            ServerSyncEvent::CancelSync(op_id) => {
                let op_id = *op_id;
                // Signal cancel via active_scripts mechanism
                if let Some(active) = self.active_scripts.get(&op_id) {
                    active.stop();
                }
                self.server_sync.update(cx, |view, _| {
                    if let Some(ref mut op) = view.active_operation {
                        if op.id == op_id {
                            op.status =
                                shelldeck_core::models::server_sync::SyncOperationStatus::Cancelled;
                        }
                    }
                });
                cx.notify();
            }
            ServerSyncEvent::SaveProfile(profile) => {
                let profile = profile.clone();
                let _ = self.store.add_sync_profile(profile.clone());
                self.server_sync.update(cx, |view, _| {
                    view.set_profiles(self.store.sync_profiles.clone());
                });
                cx.notify();
            }
            ServerSyncEvent::DeleteProfile(id) => {
                let _ = self.store.remove_sync_profile(*id);
                self.server_sync.update(cx, |view, _| {
                    view.set_profiles(self.store.sync_profiles.clone());
                    if view.selected_profile == Some(*id) {
                        view.selected_profile = None;
                    }
                });
                cx.notify();
            }
            ServerSyncEvent::ExecSync {
                source_connection_id: _,
                command: _,
                operation_id,
                item_id,
            } => {
                // Individual sync command execution — handled as part of start_sync_operation
                tracing::debug!("ExecSync for item {:?} on op {:?}", item_id, operation_id);
            }
        }
    }

    pub(super) fn list_remote_files(
        &mut self,
        connection: Connection,
        path: String,
        panel: PanelSide,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_core::models::discovery;

        let cmd = discovery::ls_command(&path);
        let fallback_cmd = discovery::ls_command_fallback(&path);
        let path_clone = path.clone();

        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<String>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<bool>();

        let spawn_result = std::thread::Builder::new()
            .name("sync-ls".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = stream_tx.send(format!("Error: async runtime: {}", e));
                        let _ = done_tx.send(false);
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match client.connect(&connection).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = stream_tx.send(format!("Error: {}", e));
                            let _ = done_tx.send(false);
                            return;
                        }
                    };

                    // Try stat-based command first
                    let output = session.exec(&cmd).await;
                    match output {
                        Ok(result) => {
                            let out = String::from_utf8_lossy(&result.stdout).to_string();
                            if !out.trim().is_empty() {
                                let _ = stream_tx.send(format!("STAT:{}", out));
                                let _ = done_tx.send(true);
                            } else {
                                // Fallback to ls
                                match session.exec(&fallback_cmd).await {
                                    Ok(result) => {
                                        let out =
                                            String::from_utf8_lossy(&result.stdout).to_string();
                                        let _ = stream_tx.send(format!("LS:{}", out));
                                        let _ = done_tx.send(true);
                                    }
                                    Err(e) => {
                                        let _ = stream_tx.send(format!("Error: {}", e));
                                        let _ = done_tx.send(false);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Fallback to ls
                            match session.exec(&fallback_cmd).await {
                                Ok(result) => {
                                    let out = String::from_utf8_lossy(&result.stdout).to_string();
                                    let _ = stream_tx.send(format!("LS:{}", out));
                                    let _ = done_tx.send(true);
                                }
                                Err(e) => {
                                    let _ = stream_tx.send(format!("Error: {}", e));
                                    let _ = done_tx.send(false);
                                }
                            }
                        }
                    }
                });
            });
        if let Err(e) = spawn_result {
            tracing::error!("Failed to spawn ls thread: {}", e);
            self.show_toast(
                t!("toast.sync.list_files_failed", error = e.to_string()).to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                if let Ok(data) = stream_rx.try_recv() {
                    let path_for_parse = path_clone.clone();
                    let entries = if let Some(stripped) = data.strip_prefix("STAT:") {
                        discovery::parse_stat_output(stripped, &path_for_parse)
                    } else if let Some(stripped) = data.strip_prefix("LS:") {
                        discovery::parse_ls_output(stripped, &path_for_parse)
                    } else {
                        // Error
                        Vec::new()
                    };

                    let _ = sync_handle.update(cx, |view, cx| {
                        view.set_file_entries(panel, path_for_parse, entries);
                        cx.notify();
                    });
                }

                if done_rx.try_recv().is_ok() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn list_local_files(
        &mut self,
        path: String,
        panel: PanelSide,
        cx: &mut Context<Self>,
    ) {
        use shelldeck_core::models::discovery;
        let entries = discovery::list_local_files(&path);
        self.server_sync.update(cx, |view, cx| {
            view.set_file_entries(panel, path, entries);
            cx.notify();
        });
    }

    pub(super) fn start_sync_operation(&mut self, profile: SyncProfile, cx: &mut Context<Self>) {
        use chrono::Utc;
        use shelldeck_core::models::discovery;
        use shelldeck_core::models::server_sync::*;

        let op_id = Uuid::new_v4();
        let item_progress: Vec<SyncProgress> = profile
            .items
            .iter()
            .map(|item| SyncProgress {
                item_id: item.id,
                status: SyncOperationStatus::Pending,
                bytes_transferred: 0,
                total_bytes: None,
                files_transferred: 0,
                total_files: None,
                current_file: None,
                error_message: None,
            })
            .collect();

        let operation = SyncOperation {
            id: op_id,
            profile_id: profile.id,
            status: SyncOperationStatus::Connecting,
            item_progress,
            log_lines: Vec::new(),
            started_at: Utc::now(),
            finished_at: None,
        };

        self.server_sync.update(cx, |view, cx| {
            view.active_operation = Some(operation);
            view.log_lines
                .push(format!("[sync] Starting sync operation {}", op_id));
            cx.notify();
        });

        // Get connection info
        let source_conn = self
            .connections
            .iter()
            .find(|c| c.id == profile.source_connection_id)
            .cloned();
        let dest_conn = self
            .connections
            .iter()
            .find(|c| c.id == profile.dest_connection_id)
            .cloned();

        let (source_conn, dest_conn) = match (source_conn, dest_conn) {
            (Some(s), Some(d)) => (s, d),
            _ => {
                self.server_sync.update(cx, |view, cx| {
                    view.append_log(
                        "[sync] Error: source or destination connection not found".to_string(),
                    );
                    if let Some(ref mut op) = view.active_operation {
                        op.status = SyncOperationStatus::Failed;
                    }
                    cx.notify();
                });
                return;
            }
        };

        // Build commands for each item
        let mut commands: Vec<(Uuid, String)> = Vec::new();
        for item in &profile.items {
            if !item.enabled {
                continue;
            }
            let cmd = match &item.kind {
                SyncItemKind::Directory {
                    source_path,
                    dest_path,
                    exclude_patterns,
                } => discovery::rsync_command(
                    source_path,
                    &dest_conn.user,
                    &dest_conn.hostname,
                    dest_path,
                    &profile.options,
                    exclude_patterns,
                ),
                SyncItemKind::Database {
                    ref name,
                    engine,
                    ref source_credentials,
                    ref dest_credentials,
                } => match engine {
                    DatabaseEngine::Mysql => discovery::mysql_sync_command(
                        name,
                        source_credentials,
                        &dest_conn.user,
                        &dest_conn.hostname,
                        dest_credentials,
                        profile.options.compress,
                    ),
                    DatabaseEngine::Postgresql => discovery::pg_sync_command(
                        name,
                        source_credentials,
                        &dest_conn.user,
                        &dest_conn.hostname,
                        dest_credentials,
                        profile.options.compress,
                    ),
                },
                SyncItemKind::NginxSite {
                    ref site,
                    ref sync_config,
                    ref sync_root,
                } => {
                    let mut cmds = Vec::new();
                    if *sync_root && !site.root.is_empty() {
                        cmds.push(discovery::rsync_command(
                            &site.root,
                            &dest_conn.user,
                            &dest_conn.hostname,
                            &site.root,
                            &profile.options,
                            &[],
                        ));
                    }
                    if *sync_config && !site.config_path.is_empty() {
                        cmds.push(discovery::rsync_command(
                            &site.config_path,
                            &dest_conn.user,
                            &dest_conn.hostname,
                            &site.config_path,
                            &profile.options,
                            &[],
                        ));
                    }
                    cmds.join(" && ")
                }
            };
            commands.push((item.id, cmd));
        }

        let total_items = commands.len();
        let (shutdown_tx, _shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<(Uuid, String)>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<(Uuid, bool)>();

        let thread_handle = std::thread::Builder::new()
            .name(format!("sync-op-{}", op_id))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = stream_tx
                            .send((Uuid::nil(), format!("[sync] async runtime error: {}", e)));
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match client.connect(&source_conn).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ =
                                stream_tx.send((Uuid::nil(), format!("[sync] SSH Error: {}", e)));
                            return;
                        }
                    };

                    for (item_id, cmd) in &commands {
                        let _ = stream_tx.send((*item_id, format!("[sync] Running: {}", cmd)));

                        let (output_tx, mut output_rx) =
                            tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

                        let fwd_tx = stream_tx.clone();
                        let fwd_item_id = *item_id;
                        let fwd_task = tokio::spawn(async move {
                            while let Some(data) = output_rx.recv().await {
                                let text = String::from_utf8_lossy(&data);
                                for line in text.lines() {
                                    let _ = fwd_tx.send((fwd_item_id, line.to_string()));
                                }
                            }
                        });

                        let (_cancel_tx, cancel_rx) = tokio::sync::mpsc::channel::<()>(1);
                        let result = session.exec_cancellable(cmd, output_tx, cancel_rx).await;
                        let _ = fwd_task.await;

                        match result {
                            Ok(_) => {
                                let _ = done_tx.send((*item_id, true));
                            }
                            Err(e) => {
                                let _ = stream_tx.send((*item_id, format!("[sync] Error: {}", e)));
                                let _ = done_tx.send((*item_id, false));
                            }
                        }
                    }
                });
            });
        let thread_handle = match thread_handle {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to spawn sync thread: {}", e);
                self.show_toast(
                    t!("toast.sync.start_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };

        self.active_scripts.insert(
            op_id,
            ActiveScript {
                shutdown_tx,
                _thread: Some(thread_handle),
            },
        );

        // UI poller
        let sync_handle = self.server_sync.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let mut all_done = std::collections::HashSet::new();

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                let mut lines = Vec::new();
                while let Ok((item_id, line)) = stream_rx.try_recv() {
                    lines.push((item_id, line));
                }

                while let Ok((item_id, success)) = done_rx.try_recv() {
                    all_done.insert(item_id);
                    let status = if success {
                        SyncOperationStatus::Completed
                    } else {
                        SyncOperationStatus::Failed
                    };
                    let _ = sync_handle.update(cx, |view, cx| {
                        if let Some(ref mut op) = view.active_operation {
                            if let Some(prog) =
                                op.item_progress.iter_mut().find(|p| p.item_id == item_id)
                            {
                                prog.status = status;
                            }
                        }
                        cx.notify();
                    });
                }

                if !lines.is_empty() {
                    let _ = sync_handle.update(cx, |view, cx| {
                        for (item_id, line) in &lines {
                            view.log_lines.push(line.clone());
                            // Parse rsync progress if applicable
                            if line.contains('%') {
                                if let Some(pct_str) =
                                    line.split_whitespace().find(|w| w.ends_with('%'))
                                {
                                    if let Ok(pct) = pct_str.trim_end_matches('%').parse::<f64>() {
                                        if let Some(ref mut op) = view.active_operation {
                                            if let Some(prog) = op
                                                .item_progress
                                                .iter_mut()
                                                .find(|p| p.item_id == *item_id)
                                            {
                                                prog.status = SyncOperationStatus::Running;
                                                prog.total_bytes = Some(100);
                                                prog.bytes_transferred = pct as u64;
                                            }
                                        }
                                    }
                                }
                            }
                            // Update current file
                            if let Some(ref mut op) = view.active_operation {
                                if let Some(prog) =
                                    op.item_progress.iter_mut().find(|p| p.item_id == *item_id)
                                {
                                    if !line.starts_with("[sync]") {
                                        prog.current_file = Some(line.clone());
                                    }
                                }
                            }
                        }
                        cx.notify();
                    });
                }

                if all_done.len() >= total_items {
                    let _ = sync_handle.update(cx, |view, cx| {
                        if let Some(ref mut op) = view.active_operation {
                            let all_success = op
                                .item_progress
                                .iter()
                                .all(|p| p.status == SyncOperationStatus::Completed);
                            op.status = if all_success {
                                SyncOperationStatus::Completed
                            } else {
                                SyncOperationStatus::Failed
                            };
                            op.finished_at = Some(Utc::now());
                        }
                        view.log_lines
                            .push("[sync] Operation complete.".to_string());
                        view.wizard_active = false;
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
    }
}
