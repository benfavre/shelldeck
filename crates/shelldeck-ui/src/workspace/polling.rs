//! Which network polls may run, decided in one pure place.
//!
//! Support, Issues, Monique and bext Cloud each poll a remote endpoint on a
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
    Monique,
    /// Monique's job count behind the Dev status-bar indicator.
    MoniqueActivity,
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
    /// `has_monique()` — a Monique endpoint resolves for this session.
    pub monique_configured: bool,
}

/// The Support console (tickets and requests) is on screen: Support mode,
/// or its view inside Dev reached from the rail. Staff accounts only.
pub(super) fn support_console_on_screen(
    mode: AppMode,
    active_view: ActiveView,
    support_allowed: bool,
) -> bool {
    support_allowed
        && match mode {
            AppMode::Support => true,
            AppMode::Dev => active_view == ActiveView::Support,
            AppMode::User => false,
        }
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
    // signed out, and both `has_monique` and `can_access_mode` need an account),
    // but implied is not testable.
    if ctx.settings_open || !ctx.signed_in {
        return false;
    }

    match surface {
        PolledSurface::Support => {
            support_console_on_screen(ctx.mode, ctx.active_view, ctx.support_allowed)
        }
        PolledSurface::Issues => {
            ctx.mode == AppMode::User
                || support_console_on_screen(ctx.mode, ctx.active_view, ctx.support_allowed)
        }
        // Monique is reachable from the User and Support home surfaces, but in Dev
        // it lives behind its own view.
        PolledSurface::Monique => {
            ctx.monique_configured
                && match ctx.mode {
                    AppMode::User | AppMode::Support => true,
                    AppMode::Dev => ctx.active_view == ActiveView::MoniqueConsole,
                }
        }
        // The status bar is on screen in every Dev view; the console view
        // has its own faster poll that already carries the jobs.
        PolledSurface::MoniqueActivity => {
            ctx.monique_configured
                && ctx.mode == AppMode::Dev
                && ctx.active_view != ActiveView::MoniqueConsole
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
            monique_configured: self.has_monique(),
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
            monique_configured: true,
        }
    }

    const ALL: [PolledSurface; 5] = [
        PolledSurface::Support,
        PolledSurface::Issues,
        PolledSurface::Monique,
        PolledSurface::MoniqueActivity,
        PolledSurface::Bext,
    ];

    // SDTEST-1059
    // SDTEST-3076: the Monique job indicator polls in every Dev view except
    // the console (which polls the jobs itself), never in User or Support
    // mode, never without a Monique endpoint.
    #[test]
    fn monique_activity_polls_in_dev_outside_the_console() {
        for view in [
            ActiveView::Terminal,
            ActiveView::Agents,
            ActiveView::Support,
        ] {
            assert!(should_poll(
                on_screen(AppMode::Dev, view),
                PolledSurface::MoniqueActivity
            ));
        }
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::MoniqueConsole),
            PolledSurface::MoniqueActivity
        ));
        assert!(should_poll(
            on_screen(AppMode::Dev, ActiveView::MoniqueConsole),
            PolledSurface::Monique
        ));
        for mode in [AppMode::User, AppMode::Support] {
            assert!(!should_poll(
                on_screen(mode, ActiveView::Terminal),
                PolledSurface::MoniqueActivity
            ));
        }
        let mut unconfigured = on_screen(AppMode::Dev, ActiveView::Terminal);
        unconfigured.monique_configured = false;
        assert!(!should_poll(unconfigured, PolledSurface::MoniqueActivity));
    }

    // SDTEST-3072: the Support console shown inside Dev (rail icons) polls
    // tickets and requests like Support mode, only while it is the Dev view
    // and only for staff; any other Dev view polls neither.
    #[test]
    fn support_console_in_dev_polls_only_while_shown() {
        let support = on_screen(AppMode::Dev, ActiveView::Support);
        assert!(should_poll(support, PolledSurface::Support));
        assert!(should_poll(support, PolledSurface::Issues));
        for view in [
            ActiveView::Terminal,
            ActiveView::Agents,
            ActiveView::Scripts,
        ] {
            let other = on_screen(AppMode::Dev, view);
            assert!(!should_poll(other, PolledSurface::Support), "{view:?}");
            assert!(!should_poll(other, PolledSurface::Issues), "{view:?}");
        }
        let mut not_staff = support;
        not_staff.support_allowed = false;
        assert!(!should_poll(not_staff, PolledSurface::Support));
        assert!(!should_poll(not_staff, PolledSurface::Issues));
        // Unchanged elsewhere: Support mode polls both, User mode requests.
        let mode = on_screen(AppMode::Support, ActiveView::Terminal);
        assert!(should_poll(mode, PolledSurface::Support));
        assert!(should_poll(mode, PolledSurface::Issues));
        let user = on_screen(AppMode::User, ActiveView::Support);
        assert!(!should_poll(user, PolledSurface::Support));
        assert!(should_poll(user, PolledSurface::Issues));
        assert!(!support_console_on_screen(
            AppMode::User,
            ActiveView::Support,
            true
        ));
        // The two rail icons: Support lands on Accueil, Requests on Demandes.
        use super::super::navigation::support_rail_target;
        use crate::sidebar::SidebarSection;
        use crate::support_view::SupportSection;
        assert_eq!(
            support_rail_target(SidebarSection::Support),
            Some(SupportSection::Home)
        );
        assert_eq!(
            support_rail_target(SidebarSection::Requests),
            Some(SupportSection::Requests)
        );
        assert_eq!(support_rail_target(SidebarSection::Terminals), None);
    }

    #[test]
    fn no_surface_polls_behind_settings_or_while_signed_out() {
        for mode in [AppMode::User, AppMode::Support, AppMode::Dev] {
            for view in [ActiveView::MoniqueConsole, ActiveView::BextCloud] {
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

        // Monique rides the User/Support home surfaces, but in Dev it only polls
        // behind its own view.
        assert!(should_poll(
            on_screen(AppMode::User, ActiveView::Dashboard),
            PolledSurface::Monique,
        ));
        assert!(should_poll(
            on_screen(AppMode::Dev, ActiveView::MoniqueConsole),
            PolledSurface::Monique,
        ));
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::Dashboard),
            PolledSurface::Monique,
        ));
        let mut unconfigured = on_screen(AppMode::User, ActiveView::Dashboard);
        unconfigured.monique_configured = false;
        assert!(!should_poll(unconfigured, PolledSurface::Monique));

        // bext Cloud is a Dev destination: its own view, nothing else.
        assert!(should_poll(
            on_screen(AppMode::Dev, ActiveView::BextCloud),
            PolledSurface::Bext,
        ));
        assert!(!should_poll(
            on_screen(AppMode::Dev, ActiveView::MoniqueConsole),
            PolledSurface::Bext,
        ));
        assert!(!should_poll(
            on_screen(AppMode::User, ActiveView::BextCloud),
            PolledSurface::Bext,
        ));
    }
}
