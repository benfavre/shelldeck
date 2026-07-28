use super::*;

impl Workspace {
    pub fn set_tray_state_publisher(&mut self, publisher: Box<dyn Fn(TrayCounters) + Send + Sync>) {
        self.tray_state_publisher = Some(publisher);
    }

    /// Wire the OS-notification dispatcher after tray init. `main.rs`
    /// supplies a closure that translates [`TrayNotification`] into a
    /// `notify-rust` call. `None` means the tray is unavailable —
    /// every subsequent emit is a no-op.
    pub fn set_tray_notifier(&mut self, notifier: Box<dyn Fn(TrayNotification) + Send + Sync>) {
        self.tray_notifier = Some(notifier);
    }

    /// Wire dynamic companion settings to the binary-level runtime.
    ///
    /// The UI crate persists the preference, while `main.rs` owns the native
    /// global-hotkey APIs. Keeping the bridge as a callback preserves that
    /// dependency boundary.
    pub fn set_companion_config_publisher(
        &mut self,
        publisher: Box<dyn Fn(CompanionConfig) + Send + Sync>,
    ) {
        self.companion_config_publisher = Some(publisher);
    }

    pub fn set_companion_shortcut_statuses(
        &mut self,
        statuses: CompanionShortcutStatuses,
        cx: &mut Context<Self>,
    ) {
        // A refused grab used to be indistinguishable from a working one: the
        // status landed in the Settings pane and nowhere else, so a global
        // shortcut that silently never registered just looked like a shortcut
        // that "rarely works". Announce the transition into a failed state
        // once, where the user actually is.
        for (kind, message) in shortcut_failure_toasts(&self.companion_shortcut_statuses, &statuses)
        {
            let _ = kind;
            self.show_toast(message, ToastLevel::Warning, cx);
        }
        self.companion_shortcut_statuses = statuses.clone();
        self.settings.update(cx, |settings, cx| {
            settings.set_companion_shortcut_statuses(statuses, cx);
        });
    }

    /// Fire an OS notification if the notifier is wired. Public so
    /// non-counter-driven events (Fleet job completion, future SSH
    /// disconnect hooks with the actual host name) can dispatch
    /// directly without going through `publish_tray_state`.
    pub fn emit_tray_notification(&self, n: TrayNotification) {
        if let Some(notifier) = self.tray_notifier.as_ref() {
            notifier(n);
        }
    }

    /// Compute current tray counters + push into the publisher AND
    /// fire OS notifications for positive deltas (new tickets, Jean
    /// pending). SSH transport loss is emitted by the individual session
    /// lifecycle because a counter cannot distinguish expected exits. The
    /// first publish just seeds `last_tray_counters` without notifying — otherwise a
    /// launch with existing unread tickets would spam the OS.
    ///
    /// Cheap enough (a few vec-scans + a small notify-rust dispatch on
    /// deltas) to call from every spot that changes user-facing state.
    /// The tray thread diffs the counters against its last known
    /// state, so redundant publishes are silently dropped.
    pub fn publish_tray_state(&mut self, cx: &App) {
        let active_ssh = self
            .connections
            .iter()
            .filter(|c| matches!(c.status, ConnectionStatus::Connected))
            .count();
        let open_tunnels = self.active_tunnels.len();
        let unread_tickets = self.support.read(cx).unread_ticket_count();
        let jean_pending = self.runtime_awaiting.len();
        let ai_tasks_running = self
            .ai_tasks
            .iter()
            .filter(|task| task.status.is_running())
            .count();
        let pinned_connections = self
            .app_config
            .pinned_connections
            .iter()
            .filter_map(|id| {
                self.connections
                    .iter()
                    .find(|connection| connection.id == *id)
                    .map(|connection| TrayPinnedConnection {
                        id: *id,
                        name: connection.display_name().to_string(),
                    })
            })
            .collect();
        let counters = TrayCounters {
            active_ssh,
            open_tunnels,
            unread_tickets,
            jean_pending,
            ai_tasks_running,
            pinned_connections,
        };

        // Delta notifications — skipped entirely on the first publish
        // so the seed value doesn't fire a startup burst. Each category
        // is opt-out via `AppConfig.tray.notify_*` (Settings → Général).
        if let Some(prev) = self.last_tray_counters.as_ref() {
            let cfg = &self.app_config.tray;
            if cfg.notify_new_tickets && counters.unread_tickets > prev.unread_tickets {
                self.emit_tray_notification(TrayNotification::NewTickets {
                    count: counters.unread_tickets - prev.unread_tickets,
                });
            }
            if cfg.notify_jean_pending && counters.jean_pending > prev.jean_pending {
                self.emit_tray_notification(TrayNotification::JeanPending {
                    count: counters.jean_pending - prev.jean_pending,
                });
            }
        }
        self.last_tray_counters = Some(counters.clone());

        if let Some(publisher) = self.tray_state_publisher.as_ref() {
            publisher(counters);
        }
    }

    pub(super) fn toggle_connection_pin(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == id)
        else {
            return;
        };
        let name = connection.display_name().to_string();
        let unpinned = self.app_config.pinned_connections.contains(&id);
        if unpinned {
            self.app_config
                .pinned_connections
                .retain(|pinned| *pinned != id);
        } else {
            self.app_config.pinned_connections.push(id);
        }

        if let Err(error) = self.app_config.save() {
            if unpinned {
                self.app_config.pinned_connections.push(id);
            } else {
                self.app_config
                    .pinned_connections
                    .retain(|pinned| *pinned != id);
            }
            tracing::error!("Failed to persist pinned connections: {error}");
            self.show_toast(
                t!("toast.connection.pin_failed", error = error.to_string()).to_string(),
                ToastLevel::Error,
                cx,
            );
            return;
        }

        self.sync_settings_config(cx);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_pinned_connections(self.app_config.pinned_connections.clone());
            cx.notify();
        });
        self.update_dashboard_stats(cx);
        self.show_toast(
            if unpinned {
                t!("toast.connection.unpinned", name = name.as_str()).to_string()
            } else {
                t!("toast.connection.pinned", name = name.as_str()).to_string()
            },
            ToastLevel::Info,
            cx,
        );
    }

    /// Connect a pinned host selected from the system tray.
    pub fn connect_pinned_connection(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.handle_sidebar_event(&SidebarEvent::ConnectionConnect(id), cx);
    }

    /// Decide whether a window-close request should proceed.
    ///
    /// When `confirm_before_close` is enabled and there are active terminal
    /// sessions or running tunnels, the first close attempt is intercepted: we
    /// warn the user and require a second close to confirm (matching the
    /// Should the close button hide the window to the tray instead of
    /// quitting? True only when the user opted in via Settings **and**
    /// the tray is actually up (no publisher = no tray, so hiding
    /// would strand the app invisible). `main.rs` checks this before
    /// `confirm_window_close` and calls `window.hide_window()` if true.
    pub fn should_hide_to_tray(&self) -> bool {
        self.app_config.tray.close_to_tray && self.tray_state_publisher.is_some()
    }

    /// app's existing two-step "click again to confirm" pattern). Returns
    /// `true` to allow the window to close, `false` to cancel.
    pub fn confirm_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.app_config.general.confirm_before_close {
            return true;
        }

        let active_terminals = self.terminal.read(cx).tab_count();
        let active_tunnels = self.active_tunnels.len();
        let active_scripts = self.active_scripts.len();
        let has_activity = active_terminals > 0 || active_tunnels > 0 || active_scripts > 0;

        if !has_activity {
            return true;
        }

        if self.pending_close_confirm {
            // Second attempt — allow the close to proceed.
            return true;
        }

        self.pending_close_confirm = true;
        // Push directly so this confirmation is shown even when general
        // notifications are disabled — the user must see why close was blocked.
        let warning = format!(
            "{} active session(s)/tunnel(s) running — close the window again to confirm exit",
            active_terminals + active_tunnels + active_scripts
        );
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(warning, ToastLevel::Warning, cx);
        });
        false
    }
}
