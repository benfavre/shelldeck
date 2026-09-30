use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::AiSurface;
use shelldeck_core::config::activity::{ActivityAction, ActivityEntry, ActivityKind};
use shelldeck_core::config::cloud_account::AppMode;
use shelldeck_core::models::port_forward::{ForwardDirection, ForwardStatus};
use shelldeck_ssh::client::SshClient;
use shelldeck_ssh::tunnel::TunnelHandle;
use uuid::Uuid;

use crate::ai_workflow::{AiNamingKind, AiWorkflowTarget};
use crate::port_forward_form::{PortForwardForm, PortForwardFormEvent};
use crate::port_forward_view::PortForwardEvent;
use crate::t;
use crate::toast::ToastLevel;

use super::{ActiveTunnel, Workspace};

impl Workspace {
    pub(super) fn handle_forward_event(
        &mut self,
        event: &PortForwardEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        match event {
            PortForwardEvent::StartForward(id) => {
                let forward_id = *id;
                tracing::info!("Start forward requested: {}", forward_id);

                // Look up the port forward configuration
                let forward = {
                    let pf_view = self.port_forwards.read(cx);
                    pf_view
                        .forwards
                        .iter()
                        .find(|f| f.id == forward_id)
                        .cloned()
                };
                let forward = match forward {
                    Some(f) => f,
                    None => {
                        tracing::error!("Port forward not found: {}", forward_id);
                        self.add_activity(
                            t!("activity.forward_not_found", id = forward_id).to_string(),
                            ActivityKind::Error,
                            cx,
                        );
                        self.show_toast(
                            t!("toast.port_forward.not_found").to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                        return;
                    }
                };

                // Don't start if already active
                if self.active_tunnels.contains_key(&forward_id)
                    || self.tunnel_starts.contains(forward_id)
                {
                    tracing::warn!("Port forward {} is already active", forward_id);
                    return;
                }

                // Look up the connection for this forward
                let connection = self
                    .connections
                    .iter()
                    .find(|c| c.id == forward.connection_id)
                    .cloned();
                let connection = match connection {
                    Some(c) => c,
                    None => {
                        tracing::error!(
                            "Connection {} not found for port forward {}",
                            forward.connection_id,
                            forward_id
                        );
                        self.port_forwards.update(cx, |pf, _| {
                            if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                                f.status = ForwardStatus::Error;
                            }
                        });
                        self.add_activity(
                            t!("activity.forward_connection_not_found").to_string(),
                            ActivityKind::Error,
                            cx,
                        );
                        self.show_toast(
                            t!("toast.forward.connection_not_found").to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                        cx.notify();
                        return;
                    }
                };

                let label = forward
                    .label
                    .clone()
                    .unwrap_or_else(|| forward.description());

                // Update status to show we're starting
                self.port_forwards.update(cx, |pf, _| {
                    if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                        f.status = ForwardStatus::Active;
                    }
                });

                self.add_activity_entry(
                    ActivityEntry::new(
                        ActivityKind::Forward,
                        t!("activity.forward_starting", label = label.as_str()).to_string(),
                    )
                    .with_target(forward_id.to_string(), label.clone())
                    .with_action(ActivityAction::OpenForward),
                    cx,
                );
                self.show_toast(
                    t!("toast.forward.starting", label = label.as_str()).to_string(),
                    ToastLevel::Info,
                    cx,
                );

                // Use a channel to send the TunnelHandle back from the background thread
                let (result_tx, result_rx) =
                    std::sync::mpsc::channel::<Result<TunnelHandle, String>>();

                let direction = forward.direction;
                let local_port = forward.local_port;
                let remote_host = forward.remote_host.clone();
                let remote_port = forward.remote_port;
                let local_host = forward.local_host.clone();

                let (attempt, thread_shutdown_tx, mut thread_shutdown_rx) =
                    self.tunnel_starts.begin(forward_id);
                let setup_shutdown_tx = thread_shutdown_tx.clone();

                // Spawn a dedicated thread with its own tokio runtime for the SSH tunnel.
                // The thread stays alive as long as the tunnel is running; the tokio runtime
                // drives the TcpListener accept loop inside start_local_forward/start_remote_forward.
                let thread_handle = std::thread::Builder::new()
                    .name(format!("tunnel-{}", forward_id))
                    .spawn(move || {
                        let rt = match tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            Ok(rt) => rt,
                            Err(e) => {
                                let msg = format!("Failed to create async runtime: {}", e);
                                tracing::error!("{}", msg);
                                let _ = result_tx.send(Err(msg));
                                return;
                            }
                        };

                        rt.block_on(async move {
                            // Cancellation covers SSH authentication as well as listener setup.
                            let setup = async {
                                let client = SshClient::new();
                                let mut session = client.connect(&connection).await?;
                                let shared_handle = session.shared_handle();
                                let mut manager = shelldeck_ssh::tunnel::TunnelManager::new();
                                let id = match direction {
                                    ForwardDirection::LocalToRemote => {
                                        manager
                                            .start_local_forward(
                                                shared_handle,
                                                local_host,
                                                local_port,
                                                remote_host,
                                                remote_port,
                                            )
                                            .await?
                                    }
                                    ForwardDirection::RemoteToLocal => {
                                        let rx =
                                            session.take_forwarded_tcpip_rx().ok_or_else(|| {
                                                shelldeck_ssh::SshError::Tunnel(
                                                    "remote forwarding channel already taken"
                                                        .to_string(),
                                                )
                                            })?;
                                        manager
                                            .start_remote_forward(
                                                shared_handle,
                                                remote_host,
                                                remote_port,
                                                local_host,
                                                local_port,
                                                rx,
                                            )
                                            .await?
                                    }
                                    ForwardDirection::Dynamic => {
                                        manager
                                            .start_socks_forward(
                                                shared_handle,
                                                local_host,
                                                local_port,
                                            )
                                            .await?
                                    }
                                };
                                Ok::<_, shelldeck_ssh::SshError>((session, manager, id))
                            };
                            let result = tokio::select! {
                                biased;
                                _ = thread_shutdown_rx.recv() => return,
                                result = setup => result,
                            };
                            match result {
                                Ok((session, manager, id)) => {
                                    let tunnel =
                                        manager.get_tunnel(&id).expect("newly started tunnel");
                                    let proxy = TunnelHandle::new_proxy(
                                        id,
                                        tunnel.status.clone(),
                                        tunnel.bytes_sent.clone(),
                                        tunnel.bytes_received.clone(),
                                        thread_shutdown_tx,
                                    );
                                    // If the UI timed out or disappeared, tear down immediately.
                                    if result_tx.send(Ok(proxy)).is_ok() {
                                        thread_shutdown_rx.recv().await;
                                    }
                                    tracing::info!("Stopping tunnels for forward {}", forward_id);
                                    manager.stop_all();
                                    // Keep the runtime alive through listener/channel cleanup and
                                    // remote cancel acknowledgement (bounded internally to 5s).
                                    let _ = tokio::time::timeout(
                                        std::time::Duration::from_secs(6),
                                        async {
                                            while manager.tunnels().iter().any(|t| t.is_active()) {
                                                tokio::time::sleep(
                                                    std::time::Duration::from_millis(10),
                                                )
                                                .await;
                                            }
                                        },
                                    )
                                    .await;
                                    let _ = tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        session.disconnect(),
                                    )
                                    .await;
                                }
                                Err(e) => {
                                    let msg = format!("Tunnel start failed: {}", e);
                                    tracing::error!("{}", msg);
                                    let _ = result_tx.send(Err(msg));
                                }
                            }
                        });
                    });
                let thread_handle = match thread_handle {
                    Ok(h) => h,
                    Err(e) => {
                        self.tunnel_starts.cancel(forward_id);
                        tracing::error!("Failed to spawn tunnel thread: {}", e);
                        self.port_forwards.update(cx, |pf, _| {
                            if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                                f.status = ForwardStatus::Error;
                            }
                        });
                        self.add_activity(
                            t!("activity.forward_start_failed", label = label.as_str()).to_string(),
                            ActivityKind::Error,
                            cx,
                        );
                        self.show_toast(
                            t!("toast.forward.start_failed", error = e.to_string()).to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                        cx.notify();
                        return;
                    }
                };

                // Now wait for the result from the background thread.
                // We use cx.spawn to avoid blocking the UI thread.
                let weak_self = cx.entity().downgrade();
                let label_for_activity = label.clone();

                cx.spawn(async move |_this, cx: &mut AsyncApp| {
                    // Wait for the result on the background executor so we don't block GPUI
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            // The SSH connection + tunnel setup happens on the dedicated thread.
                            // We give it a generous timeout.
                            result_rx.recv_timeout(std::time::Duration::from_secs(30))
                        })
                        .await;

                    if result.is_err() {
                        let _ = setup_shutdown_tx.try_send(());
                    }
                    let _ = weak_self.update(cx, |ws, cx| {
                        // Stop, retry, logout and workspace shutdown retire older callbacks.
                        if !ws.tunnel_starts.finish(forward_id, attempt) {
                            if let Ok(Ok(handle)) = result {
                                handle.stop();
                            }
                            return;
                        }
                        match result {
                            Ok(Ok(tunnel_handle)) => {
                                ws.active_tunnels.insert(
                                    forward_id,
                                    ActiveTunnel {
                                        tunnel_handle,
                                        _thread: thread_handle,
                                    },
                                );
                                ws.port_forwards.update(cx, |pf, cx| {
                                    if let Some(f) =
                                        pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                    {
                                        f.status = ForwardStatus::Active;
                                    }
                                    cx.notify();
                                });
                                ws.add_activity_entry(
                                    ActivityEntry::new(
                                        ActivityKind::Forward,
                                        t!(
                                            "activity.forward_active",
                                            label = label_for_activity.as_str()
                                        )
                                        .to_string(),
                                    )
                                    .with_target(forward_id.to_string(), label_for_activity.clone())
                                    .with_action(ActivityAction::OpenForward),
                                    cx,
                                );
                                ws.show_toast(
                                    t!("toast.forward.active", label = label_for_activity.as_str())
                                        .to_string(),
                                    ToastLevel::Success,
                                    cx,
                                );
                            }
                            error => {
                                // Also stop a worker whose result did not arrive within 30s.
                                // The receiver has gone away; a later success is never published.
                                ws.port_forwards.update(cx, |pf, cx| {
                                    if let Some(f) =
                                        pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                    {
                                        f.status = ForwardStatus::Error;
                                    }
                                    cx.notify();
                                });
                                match error {
                                    Ok(Err(err_msg)) => {
                                        ws.add_activity(
                                            t!("activity.forward_failed", error = err_msg.as_str())
                                                .to_string(),
                                            ActivityKind::Error,
                                            cx,
                                        );
                                        ws.show_toast(
                                            t!("toast.forward.failed", error = err_msg.as_str())
                                                .to_string(),
                                            ToastLevel::Error,
                                            cx,
                                        );
                                    }
                                    Err(_) => {
                                        ws.add_activity(
                                            t!(
                                                "activity.forward_timeout",
                                                label = label_for_activity.as_str()
                                            )
                                            .to_string(),
                                            ActivityKind::Error,
                                            cx,
                                        );
                                        ws.show_toast(
                                            t!(
                                                "toast.forward.timeout",
                                                label = label_for_activity.as_str()
                                            )
                                            .to_string(),
                                            ToastLevel::Warning,
                                            cx,
                                        );
                                    }
                                    Ok(Ok(_)) => unreachable!(),
                                }
                            }
                        }
                        ws.update_dashboard_stats(cx);
                        cx.notify();
                    });
                })
                .detach();

                cx.notify();
            }
            PortForwardEvent::StopForward(id) => {
                let forward_id = *id;
                tracing::info!("Stop forward requested: {}", forward_id);

                // Cancel setup before retiring an already running tunnel.
                let was_starting = self.tunnel_starts.cancel(forward_id);
                // Look up and remove the active tunnel
                if let Some(active_tunnel) = self.active_tunnels.remove(&forward_id) {
                    // Signal the tunnel to stop. This sends through the shutdown channel
                    // which causes the background thread's tokio runtime to stop the
                    // TunnelManager and exit.
                    active_tunnel.tunnel_handle.stop();

                    // Capture final byte counts before we drop the handle
                    let (final_sent, final_recv) = active_tunnel.tunnel_handle.total_bytes();

                    // Update forward status to Inactive
                    self.port_forwards.update(cx, |pf, _| {
                        if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                            f.status = ForwardStatus::Inactive;
                            f.bytes_sent = final_sent;
                            f.bytes_received = final_recv;
                        }
                    });

                    let label = {
                        let pf_view = self.port_forwards.read(cx);
                        pf_view
                            .forwards
                            .iter()
                            .find(|f| f.id == forward_id)
                            .and_then(|f| f.label.clone())
                            .unwrap_or_else(|| format!("forward {}", forward_id))
                    };

                    self.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Forward,
                            t!("activity.forward_stopped", label = label.as_str()).to_string(),
                        )
                        .with_target(forward_id.to_string(), label.clone())
                        .with_action(ActivityAction::OpenForward),
                        cx,
                    );
                    self.show_toast(
                        t!("toast.forward.stopped", label = label.as_str()).to_string(),
                        ToastLevel::Info,
                        cx,
                    );

                    tracing::info!("Port forward {} stopped", forward_id);
                } else {
                    tracing::warn!("No active tunnel found for forward {}", forward_id);

                    // Even if we don't have a tracked tunnel, reset status to Inactive
                    self.port_forwards.update(cx, |pf, _| {
                        if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                            f.status = ForwardStatus::Inactive;
                        }
                    });

                    self.add_activity(
                        if was_starting {
                            let label = self
                                .port_forwards
                                .read(cx)
                                .forwards
                                .iter()
                                .find(|f| f.id == forward_id)
                                .map(|f| f.label.clone().unwrap_or_else(|| f.description()))
                                .unwrap_or_else(|| forward_id.to_string());
                            t!("activity.forward_stopped", label = label.as_str()).to_string()
                        } else {
                            t!("activity.forward_stop_no_active").to_string()
                        },
                        ActivityKind::Forward,
                        cx,
                    );
                }

                self.update_dashboard_stats(cx);
                cx.notify();
            }
            PortForwardEvent::AddForward => {
                self.show_port_forward_form(cx);
            }
            PortForwardEvent::EditForward(id) => {
                if let Some(fwd) = self
                    .port_forwards
                    .read(cx)
                    .forwards
                    .iter()
                    .find(|f| f.id == *id)
                    .cloned()
                {
                    self.show_port_forward_form_edit(&fwd, cx);
                }
            }
            PortForwardEvent::AddPresetForward(preset) => {
                // Open the form pre-filled with preset values so the user can pick a connection
                self.show_port_forward_form_edit(preset, cx);
            }
        }
    }

    fn show_port_forward_form(&mut self, cx: &mut Context<Self>) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let ai_enabled =
            self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Naming);
        let form = cx.new(|form_cx| PortForwardForm::new(connections, ai_enabled, form_cx));

        let sub = cx.subscribe(&form, |this, form, event: &PortForwardFormEvent, cx| {
            match event {
                PortForwardFormEvent::Save(forward) => {
                    tracing::info!("Port forward created: {}", forward.description());
                    // Persist to store
                    if let Err(e) = this.store.add_port_forward(forward.clone()) {
                        tracing::error!("Failed to save port forward: {}", e);
                        this.show_toast(
                            t!("toast.forward.save_failed", error = e.to_string()).to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                    // Update the view
                    this.port_forwards.update(cx, |pf, _| {
                        pf.forwards.push(forward.clone());
                    });
                    let desc = forward.description();
                    this.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Forward,
                            t!("activity.forward_added", desc = desc.as_str()).to_string(),
                        )
                        .with_target(forward.id.to_string(), desc)
                        .with_action(ActivityAction::OpenForward),
                        cx,
                    );
                    this.show_toast(
                        t!(
                            "toast.forward.created",
                            desc = forward.description().to_string()
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
                PortForwardFormEvent::SuggestNameWithAi => {
                    // See the script form: the identity is the form instance,
                    // so a resumed task cannot rename a different one.
                    this.open_ai_workflow(
                        AiWorkflowTarget::EntityNaming {
                            kind: AiNamingKind::Tunnel,
                            target_id: form.entity_id().to_string(),
                        },
                        cx,
                    );
                }
                PortForwardFormEvent::Cancel => {
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.port_forward_form = Some(form);
        self._pf_form_sub = Some(sub);
        cx.notify();
    }

    fn show_port_forward_form_edit(
        &mut self,
        forward: &shelldeck_core::models::port_forward::PortForward,
        cx: &mut Context<Self>,
    ) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let forward = forward.clone();
        let ai_enabled =
            self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Naming);
        let form = cx.new(|form_cx| {
            PortForwardForm::from_port_forward(&forward, connections, ai_enabled, form_cx)
        });

        let sub = cx.subscribe(&form, |this, form, event: &PortForwardFormEvent, cx| {
            match event {
                PortForwardFormEvent::Save(forward) => {
                    tracing::info!("Port forward updated: {}", forward.description());
                    // Update in store
                    match this.store.update_port_forward(forward.clone()) {
                        Ok(true) => {}
                        Ok(false) => {
                            // Not found in store, add it
                            if let Err(e) = this.store.add_port_forward(forward.clone()) {
                                tracing::error!("Failed to save port forward: {}", e);
                                this.show_toast(
                                    t!("toast.forward.save_failed", error = e.to_string())
                                        .to_string(),
                                    ToastLevel::Error,
                                    cx,
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to update port forward: {}", e);
                            this.show_toast(
                                t!("toast.forward.update_failed", error = e.to_string())
                                    .to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                        }
                    }
                    // Update the view
                    this.port_forwards.update(cx, |pf, _| {
                        if let Some(existing) = pf.forwards.iter_mut().find(|f| f.id == forward.id)
                        {
                            *existing = forward.clone();
                        }
                    });
                    let desc = forward.description();
                    this.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Forward,
                            t!("activity.forward_updated", desc = desc.as_str()).to_string(),
                        )
                        .with_target(forward.id.to_string(), desc)
                        .with_action(ActivityAction::OpenForward),
                        cx,
                    );
                    this.show_toast(
                        t!(
                            "toast.forward.updated",
                            desc = forward.description().to_string()
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
                PortForwardFormEvent::SuggestNameWithAi => {
                    // See the script form: the identity is the form instance,
                    // so a resumed task cannot rename a different one.
                    this.open_ai_workflow(
                        AiWorkflowTarget::EntityNaming {
                            kind: AiNamingKind::Tunnel,
                            target_id: form.entity_id().to_string(),
                        },
                        cx,
                    );
                }
                PortForwardFormEvent::Cancel => {
                    this.port_forward_form = None;
                    this._pf_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.port_forward_form = Some(form);
        self._pf_form_sub = Some(sub);
        cx.notify();
    }
}
