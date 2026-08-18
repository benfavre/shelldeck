//! Where an AI action goes when it is staged: refused, kept as a draft, held
//! for a confirmation dialog, or executed straight away.
//!
//! `.agents/ai.md` states the contract this file makes checkable: an action may
//! only run through a typed `AiActionPlan` approved in a separate dialog, the
//! second dialog may be skipped only when a persisted policy says `Automatic`
//! *and* the risk allows it, `High` risk always confirms, `Preparation` never
//! executes, and the target and permissions are revalidated immediately before
//! execution.
//!
//! Those rules were spread across `stage_ai_action` and `confirm_ai_action`,
//! both of which need a GPUI context, so none of it could be asserted. The
//! decision is pure — it reads state and returns a route — so it lives here and
//! both gates call it.

use super::Workspace;
use shelldeck_core::ai::{
    ai_action_disposition, AiActionDisposition, AiActionPayload, AiActionRisk, AiAutonomyLevel,
    AiSurface,
};
use shelldeck_core::config::cloud_account::AppMode;

/// What staging an AI action leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AiActionRoute {
    /// The caller may not perform this action on this surface. Nothing is
    /// staged, nothing is drafted.
    Refused,
    /// Policy is `Preparation`: the plan stays a draft the user can read.
    Draft,
    /// The plan waits for an explicit confirmation dialog.
    AwaitConfirmation,
    /// Policy is `Automatic` and the risk allows it. Still routed through the
    /// same execution entry point as a confirmed plan — never a separate path.
    ExecuteWithoutDialog,
}

/// The caller's capabilities at the moment of the decision.
///
/// Sampled twice — once when staging, once immediately before executing — which
/// is what makes a plan staged in Dev unusable after a switch back to User.
#[derive(Debug, Clone, Copy)]
pub(super) struct AiActionContext {
    pub signed_in: bool,
    pub dev_allowed: bool,
    pub support_allowed: bool,
    pub jean_configured: bool,
    pub clippy_allowed: bool,
}

/// Whether this caller may perform this action at all.
///
/// Each payload names the surface that owns it: a terminal command or a script
/// belongs to Dev, a support reply to Support. Anything else is refused rather
/// than downgraded to a draft — offering a draft the caller could never run is
/// the "never display a command the caller cannot reach" mistake from
/// `.agents/roles.md`.
pub(super) fn ai_action_permitted(ctx: &AiActionContext, payload: &AiActionPayload) -> bool {
    if !ctx.signed_in {
        return false;
    }
    match payload {
        AiActionPayload::TerminalCommand { .. }
        | AiActionPayload::ScriptExecution { .. }
        | AiActionPayload::FleetDispatch { .. } => ctx.dev_allowed,
        AiActionPayload::SupportSend { .. } => ctx.support_allowed,
        AiActionPayload::JeanDispatch { .. } => ctx.jean_configured,
        AiActionPayload::ClippyReplaceSelection { .. } => ctx.clippy_allowed,
    }
}

/// The full staging decision: permission first, policy second.
///
/// Order matters. Permission is checked before the policy is consulted, so an
/// `Automatic` policy can never turn an action the caller may not perform into
/// an executed one.
pub(super) fn route_ai_action(
    ctx: &AiActionContext,
    payload: &AiActionPayload,
    level: AiAutonomyLevel,
    risk: AiActionRisk,
) -> AiActionRoute {
    if !ai_action_permitted(ctx, payload) {
        return AiActionRoute::Refused;
    }
    match ai_action_disposition(level, risk) {
        AiActionDisposition::DraftOnly => AiActionRoute::Draft,
        AiActionDisposition::Confirm => AiActionRoute::AwaitConfirmation,
        AiActionDisposition::Execute => AiActionRoute::ExecuteWithoutDialog,
    }
}

impl Workspace {
    pub(super) fn ai_action_context(&self) -> AiActionContext {
        AiActionContext {
            signed_in: self.signed_in(),
            dev_allowed: self.can_access_mode(AppMode::Dev),
            support_allowed: self.can_access_mode(AppMode::Support),
            jean_configured: self.effective_jean_config().is_some(),
            clippy_allowed: self.app_config.ai.allows(AiSurface::Clippy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shelldeck_core::ai::clippy::DesktopSelection;

    /// A selection that would pass `DesktopSelection::validate` — identity and
    /// text are required, and a password role would be rejected outright.
    fn selection() -> DesktopSelection {
        DesktopSelection {
            identity: "editor:1".into(),
            text: "selected text".into(),
            application: Some("Editor".into()),
            window_id: Some("w1".into()),
            focused_role: Some("AXTextArea".into()),
        }
    }

    fn super_admin() -> AiActionContext {
        AiActionContext {
            signed_in: true,
            dev_allowed: true,
            support_allowed: true,
            jean_configured: true,
            clippy_allowed: true,
        }
    }

    fn regular_user() -> AiActionContext {
        AiActionContext {
            signed_in: true,
            dev_allowed: false,
            support_allowed: false,
            jean_configured: false,
            clippy_allowed: true,
        }
    }

    fn terminal() -> AiActionPayload {
        AiActionPayload::TerminalCommand {
            command: "ls".into(),
        }
    }

    fn every_payload() -> Vec<AiActionPayload> {
        vec![
            terminal(),
            AiActionPayload::ScriptExecution { body: "ls".into() },
            AiActionPayload::SupportSend { body: "hi".into() },
            AiActionPayload::JeanDispatch {
                prompt: "check".into(),
            },
            AiActionPayload::FleetDispatch {
                issue_id: "i".into(),
                instance_id: "n".into(),
            },
            AiActionPayload::ClippyReplaceSelection {
                expected_selection: selection(),
                replacement: "x".into(),
            },
        ]
    }

    // SDTEST-1365 — an executable draft never runs without a second
    // confirmation, and `High` risk confirms whatever the policy says.
    #[test]
    fn only_an_automatic_policy_on_a_non_high_risk_action_skips_the_dialog() {
        let ctx = super_admin();

        for risk in [
            AiActionRisk::Low,
            AiActionRisk::Moderate,
            AiActionRisk::High,
        ] {
            assert_eq!(
                route_ai_action(&ctx, &terminal(), AiAutonomyLevel::Confirmation, risk),
                AiActionRoute::AwaitConfirmation,
                "the default policy must always open the dialog ({risk:?})",
            );
            assert_eq!(
                route_ai_action(&ctx, &terminal(), AiAutonomyLevel::Preparation, risk),
                AiActionRoute::Draft,
                "Preparation must never execute ({risk:?})",
            );
        }

        assert_eq!(
            route_ai_action(
                &ctx,
                &terminal(),
                AiAutonomyLevel::Automatic,
                AiActionRisk::Low,
            ),
            AiActionRoute::ExecuteWithoutDialog,
        );
        // The one rule an autonomy setting may not override.
        assert_eq!(
            route_ai_action(
                &ctx,
                &terminal(),
                AiAutonomyLevel::Automatic,
                AiActionRisk::High,
            ),
            AiActionRoute::AwaitConfirmation,
            "High risk confirms regardless of policy",
        );
    }

    // SDTEST-1365 — permission is decided before policy, so `Automatic` cannot
    // execute something the caller may not perform.
    #[test]
    fn an_automatic_policy_never_promotes_an_action_the_caller_cannot_perform() {
        for ctx in [
            regular_user(),
            AiActionContext {
                signed_in: false,
                ..super_admin()
            },
        ] {
            for payload in every_payload() {
                if ai_action_permitted(&ctx, &payload) {
                    continue;
                }
                assert_eq!(
                    route_ai_action(
                        &ctx,
                        &payload,
                        AiAutonomyLevel::Automatic,
                        AiActionRisk::Low,
                    ),
                    AiActionRoute::Refused,
                    "{payload:?} must be refused outright, not drafted or executed",
                );
            }
        }
    }

    // SDTEST-1365 — each payload is owned by the surface that can run it.
    #[test]
    fn each_payload_is_gated_by_the_surface_that_owns_it() {
        let mut ctx = super_admin();

        ctx.dev_allowed = false;
        assert!(!ai_action_permitted(&ctx, &terminal()));
        assert!(!ai_action_permitted(
            &ctx,
            &AiActionPayload::ScriptExecution { body: "ls".into() },
        ));
        assert!(!ai_action_permitted(
            &ctx,
            &AiActionPayload::FleetDispatch {
                issue_id: "i".into(),
                instance_id: "n".into(),
            },
        ));
        // Support and Clippy are unaffected by losing Dev.
        assert!(ai_action_permitted(
            &ctx,
            &AiActionPayload::SupportSend { body: "hi".into() },
        ));

        let mut ctx = super_admin();
        ctx.support_allowed = false;
        assert!(!ai_action_permitted(
            &ctx,
            &AiActionPayload::SupportSend { body: "hi".into() },
        ));
        assert!(ai_action_permitted(&ctx, &terminal()));

        let mut ctx = super_admin();
        ctx.jean_configured = false;
        assert!(!ai_action_permitted(
            &ctx,
            &AiActionPayload::JeanDispatch {
                prompt: "check".into(),
            },
        ));

        let mut ctx = super_admin();
        ctx.clippy_allowed = false;
        assert!(!ai_action_permitted(
            &ctx,
            &AiActionPayload::ClippyReplaceSelection {
                expected_selection: selection(),
                replacement: "x".into(),
            },
        ));
    }

    // SDTEST-1365 — a signed-out session performs no AI action at all. The
    // welcome screen intercepts rendering, but a queued plan or a global
    // shortcut must not slip past it.
    #[test]
    fn a_signed_out_session_performs_no_ai_action() {
        let ctx = AiActionContext {
            signed_in: false,
            ..super_admin()
        };
        for payload in every_payload() {
            assert!(
                !ai_action_permitted(&ctx, &payload),
                "{payload:?} was permitted while signed out",
            );
        }
    }
}
