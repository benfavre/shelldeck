//! Which network polls may run, decided in one pure place.
//!
//! Support, Issues, Jean and bext Cloud each poll a remote endpoint on a
//! timer. Each scheduler used to carry its own inline predicate, and the rules
//! they share — Settings covers the surface, a signed-out session shows the
//! welcome screen — were repeated four times with no single place to check
//! them. A fifth poll forgetting one of those rules would burn bandwidth
//! silently.
//!
//! The predicate here is pure, so the whole decision table is unit-testable
//! without a GPUI context.

use super::{ActiveView, Workspace};
use shelldeck_core::config::cloud_account::AppMode;

/// A surface that polls a remote endpoint while it is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PolledSurface {
    Support,
    Issues,
    Jean,
    Bext,
}

/// Everything the decision depends on, sampled off the workspace.
#[derive(Debug, Clone, Copy)]
pub(super) struct PollContext {
    pub settings_open: bool,
    pub mode: AppMode,
    pub active_view: ActiveView,
    pub signed_in: bool,
    /// `can_access_mode(Support)` — the account may reach the Support surface.
    pub support_allowed: bool,
    /// `has_jean()` — a Jean endpoint resolves for this session.
    pub jean_configured: bool,
}

/// Whether `surface` is on screen, and therefore worth polling.
pub(super) fn should_poll(ctx: PollContext, surface: PolledSurface) -> bool {
    // Two rules hold for every surface, so they are stated once here instead
    // of being repeated — and forgotten — per scheduler.
    //
    // Settings is a full-surface overlay: whatever was behind it is not on
    // screen any more. A signed-out session renders the welcome screen, so no
    // authenticated surface exists to refresh. The signed-out case was already
    // implied by each individual predicate (`effective_mode` forces User when
    // signed out, and both `has_jean` and `can_access_mode` need an account),
    // but implied is not testable.
    if ctx.settings_open || !ctx.signed_in {
        return false;
    }

    match surface {
        PolledSurface::Support => ctx.support_allowed && ctx.mode == AppMode::Support,
        PolledSurface::Issues => matches!(ctx.mode, AppMode::User | AppMode::Support),
        // Jean is reachable from the User and Support home surfaces, but in Dev
        // it lives behind its own view.
        PolledSurface::Jean => {
            ctx.jean_configured
                && match ctx.mode {
                    AppMode::User | AppMode::Support => true,
                    AppMode::Dev => ctx.active_view == ActiveView::JeanConsole,
                }
        }
        PolledSurface::Bext => ctx.mode == AppMode::Dev && ctx.active_view == ActiveView::BextCloud,
    }
}

impl Workspace {
    pub(super) fn poll_context(&self) -> PollContext {
        PollContext {
            settings_open: self.settings_open,
            mode: self.effective_mode(),
            active_view: self.active_view,
            signed_in: self.signed_in(),
            support_allowed: self.can_access_mode(AppMode::Support),
            jean_configured: self.has_jean(),
        }
    }

    pub(super) fn should_poll(&self, surface: PolledSurface) -> bool {
        should_poll(self.poll_context(), surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_screen(mode: AppMode, active_view: ActiveView) -> PollContext {
        PollContext {
            settings_open: false,
            mode,
            active_view,
            signed_in: true,
            support_allowed: true,
            jean_configured: true,
        }
    }

    const ALL: [PolledSurface; 4] = [
        PolledSurface::Support,
        PolledSurface::Issues,
        PolledSurface::Jean,
        PolledSurface::Bext,
    ];

    // SDTEST-1059
    #[test]
    fn no_surface_polls_behind_settings_or_while_signed_out() {
        for mode in [AppMode::User, AppMode::Support, AppMode::Dev] {
            for view in [ActiveView::JeanConsole, ActiveView::BextCloud] {
                for surface in ALL {
                    let mut ctx = on_screen(mode, view);
                    ctx.settings_open = true;
                    assert!(
                        !should_poll(ctx, surface),
                        "{surface:?} kept polling behind Settings in {mode:?}",
                    );

                    let mut ctx = on_screen(mode, view);
                    ctx.signed_in = false;
                    assert!(
                        !should_poll(ctx, surface),
                        "{surface:?} kept polling on the welcome screen in {mode:?}",
                    );
                }
            }
        }
    }

    // SDTEST-1059
    #[test]
    fn each_surface_polls_only_where_it_is_displayed() {
        // Support triage exists in Support mode only, and only for an account
        // allowed to reach it.
        assert!(should_poll(
            on_screen(AppMode::Support, ActiveView::Dashboard),
            PolledSurface::Support,
        ));
        assert!(!should_poll(
            on_screen(AppMode::User, ActiveView::Dashboard),
            PolledSurface::Support,
        ));
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::Dashboard),
            PolledSurface::Support,
        ));
        let mut denied = on_screen(AppMode::Support, ActiveView::Dashboard);
        denied.support_allowed = false;
        assert!(!should_poll(denied, PolledSurface::Support));

        // Requests are surfaced in User and Support, never in Dev.
        assert!(should_poll(
            on_screen(AppMode::User, ActiveView::Dashboard),
            PolledSurface::Issues,
        ));
        assert!(should_poll(
            on_screen(AppMode::Support, ActiveView::Dashboard),
            PolledSurface::Issues,
        ));
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::Dashboard),
            PolledSurface::Issues,
        ));

        // Jean rides the User/Support home surfaces, but in Dev it only polls
        // behind its own view.
        assert!(should_poll(
            on_screen(AppMode::User, ActiveView::Dashboard),
            PolledSurface::Jean,
        ));
        assert!(should_poll(
            on_screen(AppMode::Dev, ActiveView::JeanConsole),
            PolledSurface::Jean,
        ));
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::Dashboard),
            PolledSurface::Jean,
        ));
        let mut unconfigured = on_screen(AppMode::User, ActiveView::Dashboard);
        unconfigured.jean_configured = false;
        assert!(!should_poll(unconfigured, PolledSurface::Jean));

        // bext Cloud is a Dev destination: its own view, nothing else.
        assert!(should_poll(
            on_screen(AppMode::Dev, ActiveView::BextCloud),
            PolledSurface::Bext,
        ));
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::JeanConsole),
            PolledSurface::Bext,
        ));
        assert!(!should_poll(
            on_screen(AppMode::User, ActiveView::BextCloud),
            PolledSurface::Bext,
        ));
    }
}
