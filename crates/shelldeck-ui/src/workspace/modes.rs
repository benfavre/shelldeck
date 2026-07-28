use super::*;

impl Workspace {
    // --- App modes (User / Support / Dev) ---

    /// Whether the user is signed in to Inklura Manage.
    pub(super) fn signed_in(&self) -> bool {
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
        if !self.can_access_mode(mode) || self.app_config.cloud_sync.mode == mode {
            return;
        }
        self.app_config.cloud_sync.mode = mode;
        self.settings_open = false;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save app mode: {}", e);
        }
        self.theme_menu_open = false;
        self.account_menu_open = false;
        self.site_menu_open = false;

        // Cross-mode selection carry-over: opening a request in Support then
        // switching to User made the User-mode detail sheet auto-open on top
        // of the (unrelated) User list, because both surfaces share
        // `issue_selected`/`issue_detail`.
        self.reset_issue_selection(cx);

        self.activate_current_mode(cx);
        cx.notify();
    }

    /// Open personal Settings without destroying or replacing the current
    /// mode-specific surface.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        self.settings_open = true;
        self.theme_menu_open = false;
        self.account_menu_open = false;
        self.site_menu_open = false;
        self.activate_current_mode(cx);
        cx.notify();
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
        if self.effective_mode() == AppMode::Support && self.app_config.cloud_sync.is_configured() {
            self.refresh_support(cx);
        }
        self.update_jean_availability(cx);
        self.sync_jean_poll(cx);
        self.update_fleet_availability(cx);
        self.sync_fleet_view_poll(cx);
        self.sync_runtime_loop(cx);
        self.sync_issues_poll(cx);
    }
}
