use super::*;

impl Workspace {
    /// Pull SSH connection profiles from Inklura Manage on demand.
    ///
    /// If Cloud Sync isn't configured, this just explains how to set it up.
    /// Otherwise the blocking network fetch + merge runs on a background thread
    /// (never the UI thread), and on completion the merged connections are
    /// reloaded into the sidebar/dashboard and a toast reports the stats.
    pub fn cloud_sync_now(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        self.start_cloud_sync(true, cx);
    }

    /// Run the configured startup sync after the main Workspace exists.
    ///
    /// Unlike the manual action, this does not announce a redundant
    /// "started" toast; completion and failures remain visible.
    pub fn cloud_sync_on_startup(&mut self, cx: &mut Context<Self>) {
        if self.app_config.cloud_sync.sync_on_startup {
            self.start_cloud_sync(false, cx);
        }
    }

    pub(super) fn start_cloud_sync(&mut self, announce_started: bool, cx: &mut Context<Self>) {
        let cfg = self.app_config.cloud_sync.clone();
        if !cfg.is_configured() {
            if announce_started {
                self.show_toast(
                    t!("toast.cloud_sync.not_configured").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
            }
            return;
        }

        if announce_started {
            self.show_toast(
                t!("toast.cloud_sync.started").to_string(),
                ToastLevel::Info,
                cx,
            );
        }
        let version = shelldeck_core::VERSION;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { shelldeck_core::config::cloud_sync::sync_now(&cfg, version) })
                .await;

            let _ = this.update(cx, |ws, cx| match result {
                Ok(stats) => {
                    ws.reload_connections_after_sync(cx);
                    ws.show_toast(
                        t!(
                            "toast.cloud_sync.done",
                            added = stats.added,
                            updated = stats.updated,
                            removed = stats.removed
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
                Err(e) => {
                    ws.show_toast(
                        t!(
                            "toast.cloud_sync.failed",
                            error = crate::i18n::api_error_message(&e)
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Rebuild the in-memory connection list after Cloud Sync wrote the store.
    ///
    /// Mirrors the startup merge in `main.rs`: reload the persisted store,
    /// re-parse `~/.ssh/config`, and combine them (dedup by alias). Live
    /// connection status from the current list is carried over by id so an
    /// active session doesn't flip back to "disconnected".
    pub(super) fn reload_connections_after_sync(&mut self, cx: &mut Context<Self>) {
        let store = match ConnectionStore::load() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to reload connection store after cloud sync: {}", e);
                return;
            }
        };
        let ssh_connections =
            shelldeck_core::config::ssh_config::parse_ssh_config().unwrap_or_default();

        let mut merged = ssh_connections;
        for conn in &store.connections {
            if !merged.iter().any(|c| c.alias == conn.alias) {
                merged.push(conn.clone());
            }
        }
        // Preserve live status from the current in-memory connections.
        for m in merged.iter_mut() {
            if let Some(cur) = self.connections.iter().find(|c| c.id == m.id) {
                m.status = cur.status.clone();
            }
        }

        self.store = store;
        self.connections = merged;

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
        self.refresh_agent_connections(cx);
        self.update_dashboard_stats(cx);
        cx.notify();
    }
}
