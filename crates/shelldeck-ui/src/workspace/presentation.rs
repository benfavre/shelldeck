//! Transient launch presentation; authentication and persisted app mode remain authoritative.
use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspacePresentation {
    #[default]
    Main,
    Agents,
}

impl WorkspacePresentation {
    fn after_auth(self, signed_in: bool, dev_allowed: bool) -> Self {
        if signed_in && !dev_allowed {
            Self::Main
        } else {
            self
        }
    }

    pub(super) fn effective_mode(self, dev_allowed: bool, resolved: AppMode) -> AppMode {
        if self == Self::Agents && dev_allowed {
            AppMode::Dev
        } else {
            resolved
        }
    }
}

pub(super) fn agent_surface_visible(
    dev_allowed: bool,
    mode: AppMode,
    settings_open: bool,
    agents_selected: bool,
) -> bool {
    dev_allowed && mode == AppMode::Dev && !settings_open && agents_selected
}

impl Workspace {
    /// Request the retained production Agents surface, including before login.
    /// The welcome screen and server-issued Dev capability still gate entry.
    pub fn request_agents_presentation(&mut self, cx: &mut Context<Self>) {
        self.presentation = WorkspacePresentation::Agents;
        self.agents_focus_pending = true;
        self.settings_open = false;
        self.mode_transition = None;
        self.close_titlebar_menus();
        self.activate_current_mode(cx);
        cx.notify();
    }

    /// Restore the normal application shell without changing the saved app mode.
    pub fn show_main_presentation(&mut self, cx: &mut Context<Self>) {
        self.presentation = WorkspacePresentation::Main;
        self.agents_focus_pending = false;
        self.mode_transition = None;
        self.close_titlebar_menus();
        self.activate_current_mode(cx);
        cx.notify();
    }

    pub fn presentation(&self) -> WorkspacePresentation {
        self.presentation
    }

    pub(super) fn agents_presentation_active(&self) -> bool {
        self.presentation == WorkspacePresentation::Agents && self.can_access_mode(AppMode::Dev)
    }

    pub(super) fn reconcile_presentation(&mut self, cx: &mut Context<Self>) {
        let resolved = self
            .presentation
            .after_auth(self.signed_in(), self.can_access_mode(AppMode::Dev));
        if resolved != self.presentation {
            self.presentation = resolved;
            self.agents_focus_pending = false;
            self.show_toast(
                t!("agents.presentation.unavailable").to_string(),
                ToastLevel::Warning,
                cx,
            );
        }
        if self.agents_presentation_active() && self.agents_focus_pending {
            self.active_view = ActiveView::Agents;
            self.agents_focus_pending = false;
            self.agent_console
                .update(cx, |view, cx| view.request_surface_focus(cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{agent_surface_visible, AppMode, WorkspacePresentation};

    #[test]
    fn sdtest_2636_agents_launch_waits_for_auth_and_clamps_staff_capabilities() {
        let pending = WorkspacePresentation::Agents.after_auth(false, false);
        assert_eq!(pending, WorkspacePresentation::Agents);
        assert_eq!(pending.effective_mode(false, AppMode::User), AppMode::User);
        for allowed_mode in [AppMode::User, AppMode::Support] {
            let denied = pending.after_auth(true, false);
            assert_eq!(denied, WorkspacePresentation::Main);
            assert_eq!(denied.effective_mode(false, allowed_mode), allowed_mode);
        }
        assert_eq!(
            pending.after_auth(true, true),
            WorkspacePresentation::Agents
        );
    }

    #[test]
    fn sdtest_2637_agents_presentation_restores_saved_mode_without_persisting_dev() {
        for saved in [AppMode::User, AppMode::Support, AppMode::Dev] {
            let requested = WorkspacePresentation::Agents.after_auth(true, true);
            assert_eq!(requested.effective_mode(true, saved), AppMode::Dev);
            assert_eq!(
                WorkspacePresentation::Main.effective_mode(true, saved),
                saved
            );
            assert_eq!(
                requested.after_auth(true, false),
                WorkspacePresentation::Main
            );
        }
    }

    #[test]
    fn sdtest_2638_agents_poll_visibility_follows_auth_mode_settings_and_selection() {
        assert!(agent_surface_visible(true, AppMode::Dev, false, true));
        assert!(!agent_surface_visible(false, AppMode::Dev, false, true));
        assert!(!agent_surface_visible(true, AppMode::User, false, true));
        assert!(!agent_surface_visible(true, AppMode::Support, false, true));
        assert!(!agent_surface_visible(true, AppMode::Dev, true, true));
        assert!(!agent_surface_visible(true, AppMode::Dev, false, false));
    }
}
