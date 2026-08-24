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
use crate::ai_workflow::AiNamingKind;
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
    pub monique_configured: bool,
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
        AiActionPayload::MoniqueDispatch { .. } => ctx.monique_configured,
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

/// What a fired AI timeout should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AiTimeoutOutcome {
    /// Do nothing. Either the run already finished, or a newer action owns this
    /// target now — stopping it would kill somebody else's work.
    Ignore,
    /// Stop the run and audit it as timed out.
    StopAndAudit,
}

/// Whether a timeout that just fired still owns the run it was scheduled for.
///
/// Timers are detached and cannot be cancelled, so one always outlives its run.
/// Between the moment it was scheduled and the moment it fires, the same script
/// or terminal may already be executing a *different* AI action — or nothing at
/// all. Firing blindly would stop a newer, unrelated execution.
///
/// `tracked_action` is the action currently registered for that target, and
/// `target_still_running` lets the script path add its own liveness check; the
/// terminal path has no equivalent and passes `true`.
pub(super) fn ai_timeout_outcome(
    tracked_action: Option<uuid::Uuid>,
    scheduled_action: uuid::Uuid,
    target_still_running: bool,
) -> AiTimeoutOutcome {
    if tracked_action != Some(scheduled_action) || !target_still_running {
        return AiTimeoutOutcome::Ignore;
    }
    AiTimeoutOutcome::StopAndAudit
}

/// Where a generated name may be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NamingApplication {
    ScriptForm,
    TunnelForm,
    Terminal(uuid::Uuid),
    IssueDraft,
    /// The entity the name was generated for is closed, or a different one
    /// took its place. Applying here would rename somebody else's work.
    TargetLost,
}

/// Resolve where a generated name goes.
///
/// A naming task outlives the surface that asked for it: it can be parked and
/// resumed from the task center once a *different* form is open. The form-bound
/// kinds therefore carry the entity id of the form that asked, and this compares
/// it against the form that is open now. Before that, both carried the literal
/// `"script-form"` / `"tunnel-form"`, which named no entity at all and could
/// only ever mean "whatever is open" — the bug this closes.
pub(super) fn resolve_naming_application(
    kind: AiNamingKind,
    requested_target: &str,
    open_script_form: Option<&str>,
    open_tunnel_form: Option<&str>,
    issue_draft_open: bool,
) -> NamingApplication {
    match kind {
        AiNamingKind::Script if open_script_form == Some(requested_target) => {
            NamingApplication::ScriptForm
        }
        AiNamingKind::Tunnel if open_tunnel_form == Some(requested_target) => {
            NamingApplication::TunnelForm
        }
        // A terminal names itself by session id, which is stable for the life
        // of the session; the tab either still exists or it does not.
        AiNamingKind::Terminal => uuid::Uuid::parse_str(requested_target)
            .map(NamingApplication::Terminal)
            .unwrap_or(NamingApplication::TargetLost),
        // There is only ever one request draft sheet, so its identity is
        // simply whether it is still open.
        AiNamingKind::Issue if issue_draft_open => NamingApplication::IssueDraft,
        _ => NamingApplication::TargetLost,
    }
}

impl Workspace {
    pub(super) fn ai_action_context(&self) -> AiActionContext {
        AiActionContext {
            signed_in: self.signed_in(),
            dev_allowed: self.can_access_mode(AppMode::Dev),
            support_allowed: self.can_access_mode(AppMode::Support),
            monique_configured: self.has_monique(),
            clippy_allowed: self.app_config.ai.allows(AiSurface::Clippy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shelldeck_core::ai::clippy::DesktopSelection;
    use uuid::Uuid;

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
            monique_configured: true,
            clippy_allowed: true,
        }
    }

    fn regular_user() -> AiActionContext {
        AiActionContext {
            signed_in: true,
            dev_allowed: false,
            support_allowed: false,
            monique_configured: false,
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
            AiActionPayload::MoniqueDispatch {
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
        ctx.monique_configured = false;
        assert!(!ai_action_permitted(
            &ctx,
            &AiActionPayload::MoniqueDispatch {
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

    // SDTEST-1363 — a generated name applies only to the entity that asked.
    //
    // A naming task can be parked and resumed from the task center, by which
    // time another form may be open. Before the form carried its own entity id,
    // the target was the literal "script-form" — which named no entity at all,
    // so the name landed on whatever happened to be open.
    #[test]
    fn a_resumed_name_never_lands_on_a_different_form() {
        let asked = "form-1";
        let other = "form-2";

        assert_eq!(
            resolve_naming_application(AiNamingKind::Script, asked, Some(asked), None, false),
            NamingApplication::ScriptForm,
        );
        assert_eq!(
            resolve_naming_application(AiNamingKind::Script, asked, Some(other), None, false),
            NamingApplication::TargetLost,
            "another script form took the place of the one that asked",
        );
        assert_eq!(
            resolve_naming_application(AiNamingKind::Script, asked, None, None, false),
            NamingApplication::TargetLost,
            "the form that asked was closed",
        );

        assert_eq!(
            resolve_naming_application(AiNamingKind::Tunnel, asked, None, Some(asked), false),
            NamingApplication::TunnelForm,
        );
        assert_eq!(
            resolve_naming_application(AiNamingKind::Tunnel, asked, None, Some(other), false),
            NamingApplication::TargetLost,
        );

        // Kinds do not leak into each other: an open tunnel form cannot satisfy
        // a script naming task that happens to share its id.
        assert_eq!(
            resolve_naming_application(AiNamingKind::Script, asked, None, Some(asked), false),
            NamingApplication::TargetLost,
        );
    }

    // SDTEST-1363 — the two kinds that are not form-bound keep their own
    // identity model, and neither falls back to "whatever is open".
    #[test]
    fn terminal_names_by_session_id_and_the_request_draft_by_being_open() {
        let session = Uuid::from_u128(7);
        assert_eq!(
            resolve_naming_application(
                AiNamingKind::Terminal,
                &session.to_string(),
                None,
                None,
                false,
            ),
            NamingApplication::Terminal(session),
        );
        assert_eq!(
            resolve_naming_application(AiNamingKind::Terminal, "not-a-uuid", None, None, false),
            NamingApplication::TargetLost,
        );

        assert_eq!(
            resolve_naming_application(AiNamingKind::Issue, "any", None, None, true),
            NamingApplication::IssueDraft,
        );
        assert_eq!(
            resolve_naming_application(AiNamingKind::Issue, "any", None, None, false),
            NamingApplication::TargetLost,
            "the request draft sheet was closed",
        );
    }

    // SDTEST-1366 — a timeout may only stop the run it was scheduled for.
    //
    // The timer is detached and cannot be cancelled, so it always outlives its
    // run. Firing on whatever occupies the target at that moment would stop a
    // newer, unrelated execution — the user's script killed by a stopwatch
    // belonging to a previous one.
    #[test]
    fn a_timeout_never_stops_a_run_it_does_not_own() {
        let scheduled = Uuid::from_u128(1);
        let newer = Uuid::from_u128(2);

        assert_eq!(
            ai_timeout_outcome(Some(scheduled), scheduled, true),
            AiTimeoutOutcome::StopAndAudit,
            "its own still-running action must time out",
        );
        assert_eq!(
            ai_timeout_outcome(Some(newer), scheduled, true),
            AiTimeoutOutcome::Ignore,
            "a newer action took over this target",
        );
        assert_eq!(
            ai_timeout_outcome(None, scheduled, true),
            AiTimeoutOutcome::Ignore,
            "the run already finished and was untracked",
        );
        assert_eq!(
            ai_timeout_outcome(Some(scheduled), scheduled, false),
            AiTimeoutOutcome::Ignore,
            "the target is no longer running",
        );
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
