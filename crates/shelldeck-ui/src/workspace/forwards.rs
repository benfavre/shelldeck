use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::port_forward::{ForwardDirection, ForwardStatus};
use shelldeck_ssh::client::SshClient;
use shelldeck_ssh::tunnel::TunnelHandle;
use uuid::Uuid;

use crate::dashboard::{ActivityEvent, ActivityType};
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
                            ActivityType::Error,
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
                if self.active_tunnels.contains_key(&forward_id) {
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
                            ActivityType::Error,
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

                self.add_activity(
                    t!("activity.forward_starting", label = label.as_str()).to_string(),
                    ActivityType::Forward,
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
                            // Establish SSH connection
                            let client = SshClient::new();
                            let mut session = match client.connect(&connection).await {
                                Ok(s) => s,
                                Err(e) => {
                                    let msg = format!("SSH connection failed: {}", e);
                                    tracing::error!("{}", msg);
                                    let _ = result_tx.send(Err(msg));
                                    return;
                                }
                            };
                            tracing::info!(
                                "SSH connected for tunnel to {}",
                                connection.display_name()
                            );

                            let shared_handle = session.shared_handle();
                            let mut tunnel_manager = shelldeck_ssh::tunnel::TunnelManager::new();

                            let tunnel_result = match direction {
                                ForwardDirection::LocalToRemote => {
                                    tunnel_manager
                                        .start_local_forward(
                                            shared_handle,
                                            local_port,
                                            remote_host,
                                            remote_port,
                                        )
                                        .await
                                }
                                ForwardDirection::RemoteToLocal => {
                                    match session.take_forwarded_tcpip_rx() {
                                        Some(forwarded_rx) => {
                                            tunnel_manager
                                                .start_remote_forward(
                                                    shared_handle,
                                                    remote_port,
                                                    local_host,
                                                    local_port,
                                                    forwarded_rx,
                                                )
                                                .await
                                        }
                                        None => Err(shelldeck_ssh::SshError::Tunnel(
                                            "remote forwarding channel already taken".to_string(),
                                        )),
                                    }
                                }
                                ForwardDirection::Dynamic => {
                                    tunnel_manager
                                        .start_socks_forward(shared_handle, local_host, local_port)
                                        .await
                                }
                            };

                            match tunnel_result {
                                Ok(_tunnel_id) => {
                                    // Create a proxy shutdown channel so the UI thread can
                                    // signal this background thread to tear down the tunnel.
                                    let (thread_shutdown_tx, mut thread_shutdown_rx) =
                                        tokio::sync::mpsc::channel::<()>(1);

                                    // Build a proxy TunnelHandle that shares the real tunnel's
                                    // Arc-wrapped status and byte counters but uses the
                                    // thread-level shutdown channel.
                                    let tunnel_ref = &tunnel_manager.tunnels()
                                        [tunnel_manager.tunnels().len() - 1];
                                    let proxy_handle = TunnelHandle::new_proxy(
                                        tunnel_ref.id,
                                        tunnel_ref.status.clone(),
                                        tunnel_ref.bytes_sent.clone(),
                                        tunnel_ref.bytes_received.clone(),
                                        thread_shutdown_tx,
                                    );

                                    let _ = result_tx.send(Ok(proxy_handle));

                                    // Park this thread -- keep the tokio runtime alive so the
                                    // tunnel's spawned tasks continue to run. Wait for shutdown.
                                    thread_shutdown_rx.recv().await;

                                    // Shutdown received -- stop all tunnels and exit
                                    tracing::info!("Stopping tunnels for forward {}", forward_id);
                                    tunnel_manager.stop_all();

                                    // Give tunnel tasks a moment to clean up
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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
                        tracing::error!("Failed to spawn tunnel thread: {}", e);
                        self.port_forwards.update(cx, |pf, _| {
                            if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id) {
                                f.status = ForwardStatus::Error;
                            }
                        });
                        self.add_activity(
                            t!("activity.forward_start_failed", label = label.as_str()).to_string(),
                            ActivityType::Error,
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
                let pf_handle = self.port_forwards.downgrade();
                let dashboard_handle = self.dashboard.downgrade();
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

                    match result {
                        Ok(Ok(tunnel_handle)) => {
                            tracing::info!(
                                "Tunnel started successfully for forward {}",
                                forward_id
                            );

                            // Store the active tunnel in the workspace
                            let _ = weak_self.update(cx, |ws, cx| {
                                ws.active_tunnels.insert(
                                    forward_id,
                                    ActiveTunnel {
                                        tunnel_handle,
                                        _thread: thread_handle,
                                    },
                                );

                                // Update forward status to Active
                                ws.port_forwards.update(cx, |pf, _| {
                                    if let Some(f) =
                                        pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                    {
                                        f.status = ForwardStatus::Active;
                                    }
                                });

                                ws.add_activity(
                                    t!(
                                        "activity.forward_active",
                                        label = label_for_activity.as_str()
                                    )
                                    .to_string(),
                                    ActivityType::Forward,
                                    cx,
                                );
                                ws.show_toast(
                                    t!("toast.forward.active", label = label_for_activity.as_str())
                                        .to_string(),
                                    ToastLevel::Success,
                                    cx,
                                );
                                ws.update_dashboard_stats(cx);
                                cx.notify();
                            });
                        }
                        Ok(Err(err_msg)) => {
                            tracing::error!(
                                "Tunnel failed for forward {}: {}",
                                forward_id,
                                err_msg
                            );

                            let _ = pf_handle.update(cx, |pf, cx| {
                                if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                {
                                    f.status = ForwardStatus::Error;
                                }
                                cx.notify();
                            });

                            let _ = dashboard_handle.update(cx, |dashboard, _| {
                                dashboard.recent_activity.insert(
                                    0,
                                    ActivityEvent {
                                        icon: "alert",
                                        message: t!(
                                            "activity.forward_failed",
                                            error = err_msg.as_str()
                                        )
                                        .to_string(),
                                        timestamp: chrono::Local::now()
                                            .format("%H:%M:%S")
                                            .to_string(),
                                        event_type: ActivityType::Error,
                                    },
                                );
                                if dashboard.recent_activity.len() > 50 {
                                    dashboard.recent_activity.truncate(50);
                                }
                            });

                            let _ = weak_self.update(cx, |ws, cx| {
                                ws.show_toast(
                                    t!("toast.forward.failed", error = err_msg.as_str())
                                        .to_string(),
                                    ToastLevel::Error,
                                    cx,
                                );
                            });
                        }
                        Err(_timeout) => {
                            tracing::error!("Tunnel setup timed out for forward {}", forward_id);

                            let _ = pf_handle.update(cx, |pf, cx| {
                                if let Some(f) = pf.forwards.iter_mut().find(|f| f.id == forward_id)
                                {
                                    f.status = ForwardStatus::Error;
                                }
                                cx.notify();
                            });

                            let _ = dashboard_handle.update(cx, |dashboard, _| {
                                dashboard.recent_activity.insert(
                                    0,
                                    ActivityEvent {
                                        icon: "alert",
                                        message: t!(
                                            "activity.forward_timeout",
                                            label = label_for_activity.as_str()
                                        )
                                        .to_string(),
                                        timestamp: chrono::Local::now()
                                            .format("%H:%M:%S")
                                            .to_string(),
                                        event_type: ActivityType::Error,
                                    },
                                );
                                if dashboard.recent_activity.len() > 50 {
                                    dashboard.recent_activity.truncate(50);
                                }
                            });

                            let _ = weak_self.update(cx, |ws, cx| {
                                ws.show_toast(
                                    t!(
                                        "toast.forward.timeout",
                                        label = label_for_activity.as_str()
                                    )
                                    .to_string(),
                                    ToastLevel::Warning,
                                    cx,
                                );
                            });
                        }
                    }
                })
                .detach();

                cx.notify();
            }
            PortForwardEvent::StopForward(id) => {
                let forward_id = *id;
                tracing::info!("Stop forward requested: {}", forward_id);

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

                    self.add_activity(
                        t!("activity.forward_stopped", label = label.as_str()).to_string(),
                        ActivityType::Forward,
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
                        t!("activity.forward_stop_no_active").to_string(),
                        ActivityType::Forward,
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

        let form = cx.new(|form_cx| PortForwardForm::new(connections, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &PortForwardFormEvent, cx| {
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
                    this.add_activity(
                        t!(
                            "activity.forward_added",
                            desc = forward.description().to_string()
                        )
                        .to_string(),
                        ActivityType::Forward,
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
        let form =
            cx.new(|form_cx| PortForwardForm::from_port_forward(&forward, connections, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &PortForwardFormEvent, cx| {
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
                    this.add_activity(
                        t!(
                            "activity.forward_updated",
                            desc = forward.description().to_string()
                        )
                        .to_string(),
                        ActivityType::Forward,
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
