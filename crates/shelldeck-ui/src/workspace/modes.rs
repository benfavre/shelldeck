use super::*;

impl Workspace {
    // --- App modes (User / Support / Dev) ---

    /// Whether the user is signed in to Inklura Manage.
    pub fn signed_in(&self) -> bool {
        self.app_config.cloud_sync.is_configured() && self.app_config.account.is_some()
    }

    pub(super) fn is_superadmin(&self) -> bool {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.is_superadmin)
            .unwrap_or(false)
    }

    /// True when the account passes `isManageAdmin` server-side (inclusive
    /// of super-admin). **No longer used for mode gating** — kept only
    /// for consumers that need "is this account a CM admin?" regardless
    /// of ShellDeck-staff status.
    #[allow(dead_code)]
    pub(super) fn is_admin(&self) -> bool {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.is_admin || a.is_superadmin)
            .unwrap_or(false)
    }

    /// True when the account passes `isInkluraSupport` server-side
    /// (inclusive of super-admin). **The Support-mode gate.** `is_admin`
    /// is deliberately not used here — it would include client
    /// tenant_admins, who are customers.
    pub(super) fn is_inklura_support(&self) -> bool {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.is_inklura_support || a.is_superadmin)
            .unwrap_or(false)
    }

    /// Signed-in Inklura support OR super-admins may switch modes.
    /// Regular users and client admins see no switcher — forced User.
    pub fn can_switch_mode(&self) -> bool {
        AppMode::can_switch(
            self.signed_in(),
            self.is_inklura_support(),
            self.is_superadmin(),
        )
    }

    pub(super) fn allowed_modes(&self) -> &'static [AppMode] {
        if !self.signed_in() {
            return &[];
        }
        AppMode::allowed_modes(self.is_inklura_support(), self.is_superadmin())
    }

    pub(super) fn can_access_mode(&self, mode: AppMode) -> bool {
        self.allowed_modes().contains(&mode)
    }

    pub(super) fn enter_dev_mode(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_access_mode(AppMode::Dev) {
            return false;
        }
        let was_settings_open = self.settings_open;
        self.set_mode(AppMode::Dev, cx);
        self.settings_open = false;
        if was_settings_open {
            self.activate_current_mode(cx);
        }
        true
    }

    /// The surface to present. Logged-out → the welcome landing intercepts
    /// the render before this hits; the User fallback is defensive. Signed-
    /// in super-admin → persisted mode; inklura_support → persisted
    /// clamped to {User, Support}; anyone else (including client admins)
    /// → forced User.
    ///
    /// Delegates to `AppMode::resolve_effective`; that pure fn is under
    /// test in `SDTEST-1052`.
    pub fn effective_mode(&self) -> AppMode {
        AppMode::resolve_effective(
            self.signed_in(),
            self.is_inklura_support(),
            self.is_superadmin(),
            self.app_config.cloud_sync.mode,
        )
    }

    /// Switch to an allowed app mode. Dev surfaces are hidden, not destroyed —
    /// running terminal sessions keep going.
    pub fn set_mode(&mut self, mode: AppMode, cx: &mut Context<Self>) {
        if !self.can_access_mode(mode)
            || self.app_config.cloud_sync.mode == mode
            || self.mode_transition.is_some()
        {
            return;
        }

        // Close transient chrome immediately, but keep rendering the current
        // surface until it has faded to zero opacity.
        self.settings_open = false;
        self.account_menu_open = false;
        self.mode_menu_open = false;
        self.site_menu_open = false;
        self.reset_issue_selection(cx);
        self.mode_transition = Some(ModeTransition {
            target: mode,
            phase: ModeTransitionPhase::FadeOut,
        });
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(MODE_TRANSITION_OUT_MS))
                .await;

            let Ok(continue_transition) = this.update(cx, |workspace, cx| {
                let expected = ModeTransition {
                    target: mode,
                    phase: ModeTransitionPhase::FadeOut,
                };
                if workspace.mode_transition != Some(expected) || !workspace.can_access_mode(mode) {
                    workspace.mode_transition = None;
                    cx.notify();
                    return false;
                }

                workspace.commit_mode_change(mode, cx);
                workspace.mode_transition = Some(ModeTransition {
                    target: mode,
                    phase: ModeTransitionPhase::Loading,
                });
                cx.notify();
                true
            }) else {
                return;
            };
            if !continue_transition {
                return;
            }

            cx.background_executor()
                .timer(std::time::Duration::from_millis(MODE_TRANSITION_LOADING_MS))
                .await;
            let Ok(continue_transition) = this.update(cx, |workspace, cx| {
                let expected = ModeTransition {
                    target: mode,
                    phase: ModeTransitionPhase::Loading,
                };
                if workspace.mode_transition != Some(expected) {
                    return false;
                }
                workspace.mode_transition = Some(ModeTransition {
                    target: mode,
                    phase: ModeTransitionPhase::FadeIn,
                });
                cx.notify();
                true
            }) else {
                return;
            };
            if !continue_transition {
                return;
            }

            cx.background_executor()
                .timer(std::time::Duration::from_millis(MODE_TRANSITION_IN_MS))
                .await;
            let _ = this.update(cx, |workspace, cx| {
                let expected = ModeTransition {
                    target: mode,
                    phase: ModeTransitionPhase::FadeIn,
                };
                if workspace.mode_transition == Some(expected) {
                    workspace.mode_transition = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Commit the persisted mode only at the invisible midpoint between the
    /// outgoing and incoming surfaces.
    fn commit_mode_change(&mut self, mode: AppMode, cx: &mut Context<Self>) {
        self.app_config.cloud_sync.mode = mode;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save app mode: {}", e);
        }
        self.activate_current_mode(cx);
    }

    /// Open personal Settings without destroying or replacing the current
    /// mode-specific surface.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        self.settings_open = true;
        self.account_menu_open = false;
        self.mode_menu_open = false;
        self.site_menu_open = false;
        self.activate_current_mode(cx);
        cx.notify();
    }

    /// Open Settings directly on the character cards. The Appearance tab puts
    /// companion controls first so this route is immediately actionable.
    pub fn open_companion_settings(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        self.settings.update(cx, |settings, cx| {
            settings.set_active_tab(crate::settings::SettingsTab::Appearance, cx);
        });
        self.open_settings(cx);
    }

    /// Wipe every "which issue row is open" bit — Workspace-side selection
    /// (`issue_selected`, `issue_detail`), the User-mode sheet flags, the
    /// delete confirm dialog, AND the child `SupportView` selection — so
    /// mode switches (and any future "return to a clean list" flow) always
    /// land on an empty state. Any new issue-selection field added to
    /// `Workspace` must be reset here too.
    pub(super) fn reset_issue_selection(&mut self, cx: &mut Context<Self>) {
        self.issue_selected = None;
        self.issue_detail = None;
        self.user_new_request_sheet_open = false;
        self.user_new_request_sheet_dismissing = false;
        self.user_issue_detail_dismissing = false;
        self.confirm_issue_delete = None;
        self.confirm_attachment_delete = None;
        self.support.update(cx, |v, cx| {
            v.clear_selection();
            cx.notify();
        });
    }

    /// Start/stop the support poll and load support data for the current mode.
    /// Call after login / startup / a mode change.
    pub fn activate_current_mode(&mut self, cx: &mut Context<Self>) {
        let dev_tabs_enabled = self.can_access_mode(AppMode::Dev);
        self.settings.update(cx, |settings, cx| {
            settings.set_dev_tabs_enabled(dev_tabs_enabled, cx);
        });
        self.sync_support_poll(cx);
        if self.effective_mode() == AppMode::Support && self.can_access_mode(AppMode::Support) {
            self.refresh_support(cx);
        }
        self.update_monique_availability(cx);
        self.sync_monique_poll(cx);
        self.update_fleet_availability(cx);
        self.sync_fleet_view_poll(cx);
        self.sync_runtime_loop(cx);
        self.sync_issues_poll(cx);
    }
}
