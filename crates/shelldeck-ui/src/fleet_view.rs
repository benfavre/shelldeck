//! Native cockpit for the shared Automonique platform contract.
//!
//! The workspace performs typed client calls and returns attachments, leases,
//! or review receipts. This view prepares only actions present in the exact
//! review snapshot; the server remains the sole authority for review, Git, CI,
//! pull-request, provider-session, and delivery effects.

use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::display::badge::{Badge, BadgeVariant};
use adabraka_ui::prelude::Alert;
use gpui::prelude::*;
use gpui::*;
use std::collections::{BTreeMap, BTreeSet};

use shelldeck_core::config::platform::{
    ActionResult, Attachment, ControlClaimResult, ControlLease, PaneStreamState, PlatformAction,
    PlatformActionPreview, PlatformCockpitState, PlatformFollowUp, PlatformFollowUpResult,
    PlatformRefresh, PlatformReviewActionResult, PlatformSnapshot, PlatformText,
    ResourceCoordinate, ResourceKind, ResourceRecord, RetainedSessionRead, RetainedSessionUpdate,
    SessionHistoryEvent, SessionRecord,
};
use shelldeck_core::config::platform_review::{
    CommentAgentState, ConflictResolution, DiffSide, PlatformReviewActionPreview,
    PlatformReviewConfirmationCoordinates, PlatformReviewLoad, PlatformReviewRenderSemantic,
    PlatformReviewSemantic, PlatformReviewTarget, ReviewAction, ReviewAnchorSemantic,
    ReviewProposalKind, ReviewReceiptOutcome,
};

use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum FleetViewEvent {
    Refresh,
    Attach(ResourceCoordinate),
    Detach(ResourceCoordinate),
    ClaimControl(ResourceCoordinate),
    ReleaseControl(ResourceCoordinate, ControlLease),
    Execute(PlatformActionPreview),
    ExecuteReview(PlatformReviewActionPreview),
    FollowUp(PlatformFollowUp),
}

impl EventEmitter<FleetViewEvent> for FleetView {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReviewDispatchDirective {
    Idle,
    ExecuteOnce,
    LookupOnly,
}

fn review_dispatch_directive(has_prepared: bool, has_unresolved: bool) -> ReviewDispatchDirective {
    if has_unresolved {
        ReviewDispatchDirective::LookupOnly
    } else if has_prepared {
        ReviewDispatchDirective::ExecuteOnce
    } else {
        ReviewDispatchDirective::Idle
    }
}

fn exact_review_target_index<T>(
    items: &[T],
    target: Option<&PlatformReviewTarget>,
    target_of: impl Fn(&T) -> &PlatformReviewTarget,
) -> Option<usize> {
    let target = target?;
    items.iter().position(|item| target_of(item) == target)
}

fn same_exact_review_snapshot(
    current_target: Option<&PlatformReviewTarget>,
    current_revision: Option<u64>,
    next_target: Option<&PlatformReviewTarget>,
    next_revision: Option<u64>,
) -> bool {
    current_target == next_target && current_revision.is_some() && current_revision == next_revision
}

fn localized_review_proposal_kind(kind: ReviewProposalKind) -> String {
    match kind {
        ReviewProposalKind::Stage => t!("fleet.review.action_stage").to_string(),
        ReviewProposalKind::Unstage => t!("fleet.review.action_unstage").to_string(),
        ReviewProposalKind::Commit => t!("fleet.review.action_commit").to_string(),
        ReviewProposalKind::ResolveConflict => {
            t!("fleet.review.action_resolve_conflict").to_string()
        }
    }
}

fn localized_conflict_resolution(value: ConflictResolution) -> String {
    match value {
        ConflictResolution::KeepCurrent => t!("fleet.review.resolution_keep_current").to_string(),
        ConflictResolution::KeepIncoming => t!("fleet.review.resolution_keep_incoming").to_string(),
    }
}

fn localized_external_review_coordinates(
    value: PlatformReviewConfirmationCoordinates,
) -> (String, String) {
    match value {
        PlatformReviewConfirmationCoordinates::Comment {
            comment_id,
            revision,
        } => (
            t!("fleet.review.action_send_comment").to_string(),
            t!(
                "fleet.review.coordinate_comment",
                id = comment_id,
                revision = revision.get()
            )
            .to_string(),
        ),
        PlatformReviewConfirmationCoordinates::CommentBatch { comments } => (
            t!("fleet.review.action_send_comments").to_string(),
            comments
                .into_iter()
                .map(|(id, revision)| {
                    t!(
                        "fleet.review.coordinate_comment",
                        id = id,
                        revision = revision.get()
                    )
                    .to_string()
                })
                .collect::<Vec<_>>()
                .join(", "),
        ),
        PlatformReviewConfirmationCoordinates::Proposal { kind, proposal_id } => (
            localized_review_proposal_kind(kind),
            t!("fleet.review.coordinate_proposal", id = proposal_id).to_string(),
        ),
        PlatformReviewConfirmationCoordinates::ConflictResolution {
            proposal_id,
            file_id,
            resolution,
        } => (
            t!("fleet.review.action_resolve_conflict").to_string(),
            t!(
                "fleet.review.coordinate_conflict",
                proposal = proposal_id,
                file = file_id,
                resolution = localized_conflict_resolution(resolution)
            )
            .to_string(),
        ),
        PlatformReviewConfirmationCoordinates::Check { check_id, revision } => (
            t!("fleet.review.action_rerun_check").to_string(),
            t!(
                "fleet.review.coordinate_check",
                id = check_id,
                revision = revision.get()
            )
            .to_string(),
        ),
        PlatformReviewConfirmationCoordinates::PullRequestOpen { revision, title } => (
            t!("fleet.review.action_open_pull_request").to_string(),
            t!(
                "fleet.review.coordinate_pull_request_open",
                revision = revision.get(),
                title = title
            )
            .to_string(),
        ),
        PlatformReviewConfirmationCoordinates::PullRequestUpdate {
            pull_request_id,
            revision,
            title,
        } => (
            t!("fleet.review.action_update_pull_request").to_string(),
            t!(
                "fleet.review.coordinate_pull_request_update",
                id = pull_request_id,
                revision = revision.get(),
                title = title
            )
            .to_string(),
        ),
        PlatformReviewConfirmationCoordinates::PullRequestMerge {
            pull_request_id,
            revision,
            head_revision,
        } => (
            t!("fleet.review.action_merge_pull_request").to_string(),
            t!(
                "fleet.review.coordinate_pull_request_merge",
                id = pull_request_id,
                revision = revision.get(),
                head = head_revision
            )
            .to_string(),
        ),
    }
}

pub struct FleetView {
    snapshot: Option<PlatformSnapshot>,
    review: Option<PlatformReviewLoad>,
    review_target: Option<PlatformReviewTarget>,
    cockpit: PlatformCockpitState,
    search_state: Entity<InputState>,
    search_query: String,
    composer_state: Entity<InputState>,
    composer_value: String,
    review_comment_state: Entity<InputState>,
    review_comment_value: String,
    selected_review_anchor: Option<ReviewAnchorSemantic>,
    selected_review_comments: BTreeSet<String>,
    pending_review_preview: Option<PlatformReviewActionPreview>,
    unresolved_review_actions: Vec<PlatformReviewActionPreview>,
    review_receipt: Option<(String, String)>,
    drafts: BTreeMap<String, String>,
    selected_session: Option<String>,
    pending_action: Option<PlatformActionPreview>,
    refusal: Option<(String, String)>,
    loading: bool,
    operation_busy: bool,
    error: Option<String>,
}

impl FleetView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: None,
            review: None,
            review_target: None,
            cockpit: PlatformCockpitState::default(),
            search_state: cx.new(InputState::new),
            search_query: String::new(),
            composer_state: cx.new(InputState::new),
            composer_value: String::new(),
            review_comment_state: cx.new(InputState::new),
            review_comment_value: String::new(),
            selected_review_anchor: None,
            selected_review_comments: BTreeSet::new(),
            pending_review_preview: None,
            unresolved_review_actions: Vec::new(),
            review_receipt: None,
            drafts: BTreeMap::new(),
            selected_session: None,
            pending_action: None,
            refusal: None,
            loading: false,
            operation_busy: false,
            error: None,
        }
    }

    pub fn set_snapshot(&mut self, snapshot: PlatformSnapshot) {
        self.cockpit.retain_directory_sessions(&snapshot.sessions);
        self.snapshot = Some(snapshot);
        self.cockpit.mark_online();
        self.loading = false;
        self.error = None;
    }

    pub fn apply_refresh(&mut self, refresh: PlatformRefresh) {
        for attachment in refresh.attachments {
            self.cockpit.apply_attachment_refresh(attachment);
        }
        self.set_snapshot(refresh.snapshot);
    }

    pub fn set_review(
        &mut self,
        target: Option<PlatformReviewTarget>,
        review: Option<PlatformReviewLoad>,
    ) {
        let prior_revision = match &self.review {
            Some(PlatformReviewLoad::Available(review)) => Some(review.revision.get()),
            _ => None,
        };
        let next_revision = match &review {
            Some(PlatformReviewLoad::Available(review)) => Some(review.revision.get()),
            _ => None,
        };
        let same_exact_snapshot = same_exact_review_snapshot(
            self.review_target.as_ref(),
            prior_revision,
            target.as_ref(),
            next_revision,
        );
        if !same_exact_snapshot {
            self.selected_review_anchor = None;
            self.selected_review_comments.clear();
        }
        let selected_is_current = match (&review, &self.selected_review_anchor) {
            (Some(PlatformReviewLoad::Available(review)), Some(anchor)) => review
                .files
                .iter()
                .find(|file| file.id == anchor.file_id)
                .and_then(|file| file.hunks.iter().find(|hunk| hunk.id == anchor.hunk_id))
                .is_some(),
            (_, None) => true,
            _ => false,
        };
        if !selected_is_current {
            self.selected_review_anchor = None;
        }
        match &review {
            Some(PlatformReviewLoad::Available(review)) => {
                self.selected_review_comments
                    .retain(|id| review.comment_is_batch_actionable(id));
            }
            _ => self.selected_review_comments.clear(),
        }
        if self.pending_review_preview.as_ref().is_some_and(|preview| {
            target.as_ref() != Some(preview.target())
                || !matches!(&review, Some(PlatformReviewLoad::Available(value)) if value.revision == preview.expected_revision())
        }) {
            self.pending_review_preview = None;
        }
        self.review_target = target;
        self.review = review;
    }

    pub fn pending_review_reconciliation(&self) -> Option<PlatformReviewActionPreview> {
        let unresolved = self.current_unresolved_review_action();
        (review_dispatch_directive(self.pending_review_preview.is_some(), unresolved.is_some())
            == ReviewDispatchDirective::LookupOnly)
            .then(|| unresolved.cloned())
            .flatten()
    }

    fn current_unresolved_review_action(&self) -> Option<&PlatformReviewActionPreview> {
        exact_review_target_index(
            &self.unresolved_review_actions,
            self.review_target.as_ref(),
            PlatformReviewActionPreview::target,
        )
        .and_then(|index| self.unresolved_review_actions.get(index))
    }

    pub fn retained_reads(&self) -> Vec<RetainedSessionRead> {
        self.cockpit.retained_reads()
    }

    pub fn pending_follow_ups(&self) -> Vec<PlatformFollowUp> {
        self.cockpit.pending_follow_ups()
    }

    pub fn apply_retained_updates(&mut self, updates: Vec<RetainedSessionUpdate>) {
        for update in updates {
            self.cockpit.apply_retained_update(update);
        }
    }

    pub fn set_follow_up_result(&mut self, result: PlatformFollowUpResult, cx: &mut Context<Self>) {
        match &result {
            PlatformFollowUpResult::Receipt { follow_up, receipt } => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.view.apply_receipt(receipt.clone());
                    snapshot.resources = snapshot.view.resources().cloned().collect();
                }
                if matches!(
                    receipt.outcome,
                    shelldeck_core::config::platform::ReceiptOutcome::Accepted
                        | shelldeck_core::config::platform::ReceiptOutcome::Completed
                ) {
                    self.drafts.remove(follow_up.request.session.id.as_str());
                    if self.selected_session.as_deref()
                        == Some(follow_up.request.session.id.as_str())
                    {
                        self.composer_value.clear();
                        self.composer_state.update(cx, |state, cx| state.reset(cx));
                    }
                }
            }
            PlatformFollowUpResult::Refused {
                follow_up: _,
                outcome,
                explanation,
            } => {
                self.refusal = Some((outcome.as_str().to_owned(), explanation.as_str().to_owned()));
            }
            PlatformFollowUpResult::ReconciliationPending { .. } => {}
        }
        self.cockpit.apply_follow_up_result(&result);
        self.operation_busy = false;
        self.loading = false;
    }

    pub fn prepare_selected_follow_up(
        &mut self,
    ) -> shelldeck_core::error::Result<PlatformFollowUp> {
        let session = self
            .cockpit
            .selected()
            .map(|pane| pane.attachment.session.clone())
            .ok_or_else(|| {
                shelldeck_core::error::ShellDeckError::Connection(
                    "no retained session is selected".to_string(),
                )
            })?;
        self.cockpit
            .prepare_follow_up(&session, self.composer_value.trim().to_owned())
    }

    fn select_session(&mut self, session: &ResourceCoordinate, cx: &mut Context<Self>) {
        if let Some(current) = self.selected_session.as_ref() {
            self.drafts
                .insert(current.clone(), self.composer_value.clone());
        }
        self.selected_session = Some(session.id.as_str().to_owned());
        self.cockpit.select(session);
        self.composer_value = self
            .drafts
            .get(session.id.as_str())
            .cloned()
            .unwrap_or_default();
        let value = self.composer_value.clone();
        self.composer_state.update(cx, |state, cx| {
            state.content = value.into();
            cx.notify();
        });
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.refusal = None;
        self.cockpit.mark_offline();
        self.loading = false;
    }

    pub fn set_operation_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.refusal = None;
        self.operation_busy = false;
        self.loading = false;
    }

    pub fn begin_operation(&mut self) -> bool {
        if self.operation_busy || self.loading {
            return false;
        }
        self.operation_busy = true;
        true
    }

    pub fn can_refresh(&self) -> bool {
        !self.operation_busy
    }

    pub fn set_attached(&mut self, attachment: Attachment, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.view.track_attachment(&attachment);
        }
        let session = attachment.session.clone();
        self.cockpit.attach(attachment);
        self.select_session(&session, cx);
        self.operation_busy = false;
        self.refusal = None;
        self.error = None;
    }

    pub fn set_detached(&mut self, session: &ResourceCoordinate) {
        if let Some(attachment) = self.cockpit.detach(session) {
            if let Some(snapshot) = self.snapshot.as_mut() {
                snapshot.view.forget_attachment(&attachment);
            }
        }
        self.operation_busy = false;
        self.refusal = None;
        self.error = None;
    }

    pub fn set_control_lease(&mut self, lease: ControlLease) {
        self.cockpit.set_lease(lease);
        self.operation_busy = false;
        self.refusal = None;
        self.error = None;
    }

    pub fn set_control_claim_result(&mut self, result: ControlClaimResult) {
        match result {
            ControlClaimResult::Claimed(lease) => self.set_control_lease(lease),
            ControlClaimResult::Refused {
                outcome,
                explanation,
            } => {
                self.refusal = Some((outcome.as_str().to_owned(), explanation.as_str().to_owned()));
                self.error = None;
                self.operation_busy = false;
            }
        }
    }

    pub fn set_control_released(&mut self, session: &ResourceCoordinate) {
        self.cockpit.release_lease(session);
        self.operation_busy = false;
        self.refusal = None;
        self.error = None;
    }

    pub fn set_action_result(&mut self, result: ActionResult) {
        match result {
            ActionResult::Receipt(receipt) => {
                self.pending_action = None;
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.view.apply_receipt(receipt);
                    snapshot.resources = snapshot.view.resources().cloned().collect();
                }
                self.refusal = None;
                self.error = None;
            }
            ActionResult::Refused {
                outcome,
                explanation,
            } => {
                if outcome != shelldeck_core::config::platform::ReceiptOutcome::Unknown {
                    self.pending_action = None;
                }
                self.refusal = Some((outcome.as_str().to_owned(), explanation.as_str().to_owned()));
                self.error = None;
            }
        }
        self.operation_busy = false;
        self.loading = false;
    }

    fn available_review(&self) -> Result<&PlatformReviewSemantic, &'static str> {
        match self.review.as_ref() {
            Some(PlatformReviewLoad::Available(review)) => Ok(review),
            _ => Err("review projection is unavailable"),
        }
    }

    fn prepare_review_comment(&mut self) -> Result<(), &'static str> {
        let target = self
            .review_target
            .clone()
            .ok_or("exact review target is unavailable")?;
        let anchor = self
            .selected_review_anchor
            .clone()
            .ok_or("exact review anchor is unavailable")?;
        let body = self.review_comment_value.clone();
        let preview = PlatformReviewActionPreview::add_comment(
            target,
            self.available_review()?,
            &anchor,
            &body,
        )?;
        self.pending_review_preview = Some(preview);
        Ok(())
    }

    fn prepare_review_approval(&mut self) -> Result<(), &'static str> {
        let target = self
            .review_target
            .clone()
            .ok_or("exact review target is unavailable")?;
        let preview = PlatformReviewActionPreview::approve(target, self.available_review()?)?;
        self.pending_review_preview = Some(preview);
        Ok(())
    }

    fn prepare_review_comment_batch(&mut self) -> Result<(), &'static str> {
        let target = self
            .review_target
            .clone()
            .ok_or("exact review target is unavailable")?;
        let selected = self
            .selected_review_comments
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let preview = PlatformReviewActionPreview::batch_send_comments(
            target,
            self.available_review()?,
            &selected,
        )?;
        self.pending_review_preview = Some(preview);
        Ok(())
    }

    fn prepare_review_proposal(&mut self, proposal_id: &str) -> Result<(), &'static str> {
        let target = self
            .review_target
            .clone()
            .ok_or("exact review target is unavailable")?;
        let preview = PlatformReviewActionPreview::apply_proposal(
            target,
            self.available_review()?,
            proposal_id,
        )?;
        self.pending_review_preview = Some(preview);
        Ok(())
    }

    fn prepare_review_check_rerun(&mut self, check_id: &str) -> Result<(), &'static str> {
        let target = self
            .review_target
            .clone()
            .ok_or("exact review target is unavailable")?;
        let preview =
            PlatformReviewActionPreview::rerun_check(target, self.available_review()?, check_id)?;
        self.pending_review_preview = Some(preview);
        Ok(())
    }

    fn prepare_review_merge(&mut self) -> Result<(), &'static str> {
        let target = self
            .review_target
            .clone()
            .ok_or("exact review target is unavailable")?;
        let preview =
            PlatformReviewActionPreview::merge_pull_request(target, self.available_review()?)?;
        self.pending_review_preview = Some(preview);
        Ok(())
    }

    fn dispatch_review_preview(&mut self) -> Option<PlatformReviewActionPreview> {
        let has_unresolved = self.current_unresolved_review_action().is_some();
        if self.operation_busy
            || review_dispatch_directive(self.pending_review_preview.is_some(), has_unresolved)
                != ReviewDispatchDirective::ExecuteOnce
        {
            return None;
        }
        let preview = self.pending_review_preview.take()?;
        self.operation_busy = true;
        self.unresolved_review_actions.push(preview.clone());
        self.review_receipt = Some((
            t!("fleet.review.state_pending").to_string(),
            t!("fleet.review.pending_detail").to_string(),
        ));
        Some(preview)
    }

    pub fn set_review_action_result(
        &mut self,
        result: PlatformReviewActionResult,
        cx: &mut Context<Self>,
    ) {
        let keep_lookup = result.requires_lookup();
        let preview = result.preview().clone();
        match result {
            PlatformReviewActionResult::Receipt { receipt, .. } => {
                self.review_receipt = Some((
                    receipt.outcome().as_str().to_owned(),
                    format!(
                        "{} · {}",
                        receipt.action_id().as_str(),
                        receipt.reconciliation().as_str()
                    ),
                ));
                if matches!(receipt.outcome(), ReviewReceiptOutcome::Completed)
                    && matches!(preview.action(), ReviewAction::AddComment { .. })
                {
                    self.review_comment_value.clear();
                    self.review_comment_state
                        .update(cx, |state, cx| state.reset(cx));
                }
                if matches!(receipt.outcome(), ReviewReceiptOutcome::Completed)
                    && matches!(
                        preview.action(),
                        ReviewAction::BatchSendCommentsToAgent { .. }
                    )
                {
                    self.selected_review_comments.clear();
                }
            }
            PlatformReviewActionResult::Refused {
                category,
                explanation,
                ..
            } => {
                self.review_receipt = Some((category, explanation));
            }
            PlatformReviewActionResult::ReconciliationPending { category, .. } => {
                self.review_receipt =
                    Some((t!("fleet.review.state_ambiguous").to_string(), category));
            }
        }
        self.unresolved_review_actions
            .retain(|candidate| candidate.idempotency_key() != preview.idempotency_key());
        if keep_lookup {
            self.unresolved_review_actions.push(preview);
        }
        self.operation_busy = false;
        self.loading = false;
    }

    pub fn reset_review_action_for_context_change(&mut self) {
        self.pending_review_preview = None;
        self.review_receipt = None;
        self.operation_busy = false;
    }

    pub fn attachments(&self) -> Vec<Attachment> {
        self.cockpit.attachments().cloned().collect()
    }

    pub fn attachment(&self, session: &ResourceCoordinate) -> Option<Attachment> {
        self.cockpit
            .pane(session)
            .map(|pane| pane.attachment.clone())
    }

    pub fn reset(&mut self) {
        self.snapshot = None;
        self.review = None;
        self.review_target = None;
        self.cockpit = PlatformCockpitState::default();
        self.search_query.clear();
        self.composer_value.clear();
        self.drafts.clear();
        self.selected_session = None;
        self.pending_action = None;
        self.review_comment_value.clear();
        self.selected_review_anchor = None;
        self.selected_review_comments.clear();
        self.pending_review_preview = None;
        self.unresolved_review_actions.clear();
        self.review_receipt = None;
        self.refusal = None;
        self.loading = false;
        self.operation_busy = false;
        self.error = None;
    }

    pub fn open_session_by_id(&mut self, session_id: &str) -> bool {
        let exists = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .sessions
                .iter()
                .any(|session| session.session.resource.id.as_str() == session_id)
        });
        if exists {
            self.selected_session = Some(session_id.to_owned());
            if let Some(session) = self.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .sessions
                    .iter()
                    .find(|session| session.session.resource.id.as_str() == session_id)
            }) {
                self.cockpit.select(&session.session.resource);
            }
        }
        exists
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let (resources, sessions, models, receipts, methods) =
            self.snapshot.as_ref().map_or((0, 0, 0, 0, 0), |snapshot| {
                (
                    snapshot.resources.len(),
                    snapshot.sessions.len(),
                    snapshot
                        .resources
                        .iter()
                        .filter(|resource| resource.resource.kind == ResourceKind::Model)
                        .count(),
                    snapshot.view.receipts().len().max(
                        snapshot
                            .resources
                            .iter()
                            .filter(|resource| resource.resource.kind == ResourceKind::Receipt)
                            .count(),
                    ),
                    snapshot.capabilities.methods.len(),
                )
            });
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("fleet.title").to_string()),
                    )
                    .children([
                        Badge::new(if self.cockpit.is_online() {
                            t!("fleet.connection.online").to_string()
                        } else {
                            t!("fleet.connection.offline").to_string()
                        })
                        .variant(if self.cockpit.is_online() {
                            BadgeVariant::Default
                        } else {
                            BadgeVariant::Destructive
                        }),
                        Badge::new(t!("fleet.metric.resources", count = resources).to_string())
                            .variant(BadgeVariant::Outline),
                        Badge::new(t!("fleet.metric.sessions", count = sessions).to_string())
                            .variant(BadgeVariant::Outline),
                        Badge::new(t!("fleet.metric.models", count = models).to_string())
                            .variant(BadgeVariant::Secondary),
                        Badge::new(t!("fleet.metric.receipts", count = receipts).to_string())
                            .variant(BadgeVariant::Secondary),
                        Badge::new(t!("fleet.metric.methods", count = methods).to_string())
                            .variant(BadgeVariant::Outline),
                    ]),
            )
            .child(
                Button::new("platform-refresh", t!("fleet.refresh").to_string())
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .h(px(32.0))
                    .icon(IconSource::from("refresh-cw"))
                    .loading(self.loading)
                    .disabled(self.loading || self.operation_busy)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |_this, cx| cx.emit(FleetViewEvent::Refresh));
                    }),
            )
    }

    fn render_review_summary(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(review) = self.review.as_ref() else {
            return div().into_any_element();
        };
        match review {
            PlatformReviewLoad::Available(review) => self.render_available_review(review, cx),
            PlatformReviewLoad::Unavailable(unavailable) => div()
                .px(px(12.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    Alert::warning()
                        .title(
                            t!(
                                "fleet.review.unavailable",
                                category = unavailable.category.as_str()
                            )
                            .to_string(),
                        )
                        .description(unavailable.explanation.clone()),
                )
                .into_any_element(),
        }
    }

    fn render_review_badges(review: &PlatformReviewSemantic) -> AnyElement {
        let attention = &review.attention;
        let render = PlatformReviewRenderSemantic::from(review);
        let attention_variant = if attention.needs_user_action {
            BadgeVariant::Warning
        } else {
            match attention.state {
                shelldeck_core::config::platform_review::AttentionState::Blocked => {
                    BadgeVariant::Destructive
                }
                shelldeck_core::config::platform_review::AttentionState::Done => {
                    BadgeVariant::Default
                }
                _ => BadgeVariant::Secondary,
            }
        };
        let hunk_count = review
            .files
            .iter()
            .map(|file| file.hunks.len())
            .sum::<usize>();
        let conflict_count = review
            .files
            .iter()
            .filter(|file| {
                file.conflict != shelldeck_core::config::platform_review::ConflictState::None
            })
            .count();
        let unread_comments = review
            .comments
            .iter()
            .filter(|comment| comment.unread)
            .count();
        let source_revision = render
            .attention
            .source_revision
            .map_or_else(|| "—".to_owned(), |revision| revision.get().to_string());
        let reason = render
            .attention
            .reason_key
            .as_deref()
            .map_or_else(|| "—".to_owned(), semantic_words);
        let authority =
            |value: &shelldeck_core::config::platform_review::ReviewAuthoritySemantic| {
                format!("{}:{}", value.kind.as_str(), value.id)
            };
        let checks = render
            .checks
            .iter()
            .zip(review.checks.iter())
            .map(|(semantic, check)| {
                format!(
                    "{}={} · {}@{} · {}",
                    semantic.id,
                    semantic_words(&semantic.semantic.semantic_key),
                    semantic_words(&semantic.semantic.freshness_key),
                    semantic.semantic.source_revision.get(),
                    authority(&check.authority)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let previews = render
            .previews
            .iter()
            .map(|preview| {
                format!(
                    "{}={}@{}",
                    preview.id,
                    semantic_words(&preview.semantic_key),
                    preview.source_revision.get()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        div()
            .id("platform-review-summary")
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .overflow_x_scroll()
            .child(
                Badge::new(
                    t!(
                        "fleet.review.attention",
                        state = semantic_words(&render.attention.semantic_key),
                        reason = reason,
                        revision = source_revision,
                        unread = attention.unread
                    )
                    .to_string(),
                )
                .variant(attention_variant),
            )
            .child(
                Badge::new(
                    t!(
                        "fleet.review.status",
                        decision = semantic_words(&render.review.semantic_key),
                        freshness = semantic_words(&render.review.freshness_key),
                        revision = render.review.source_revision.get(),
                        authority = authority(&review.review.authority)
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Outline),
            )
            .child(
                Badge::new(
                    t!(
                        "fleet.review.pull_request",
                        semantic = semantic_words(&render.pull_request.semantic_key),
                        freshness = semantic_words(&render.pull_request.freshness_key),
                        revision = render.pull_request.source_revision.get(),
                        authority = authority(&review.pull_request.authority)
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Outline),
            )
            .child(
                Badge::new(
                    t!(
                        "fleet.review.delivery",
                        state = semantic_words(&render.delivery.semantic_key),
                        freshness = semantic_words(&render.delivery.freshness_key),
                        revision = render.delivery.source_revision.get(),
                        authority = authority(&review.delivery.authority)
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Outline),
            )
            .child(
                Badge::new(
                    t!(
                        "fleet.review.checks",
                        count = review.checks.len(),
                        checks = checks
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Secondary),
            )
            .child(
                Badge::new(
                    t!(
                        "fleet.review.previews",
                        count = render.previews.len(),
                        previews = previews
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Secondary),
            )
            .child(
                Badge::new(
                    t!(
                        "fleet.review.diff",
                        files = review.files.len(),
                        hunks = hunk_count,
                        conflicts = conflict_count,
                        comments = review.comments.len(),
                        unread = unread_comments
                    )
                    .to_string(),
                )
                .variant(BadgeVariant::Secondary),
            )
            .into_any_element()
    }

    fn render_available_review(
        &self,
        review: &PlatformReviewSemantic,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let unresolved = self.current_unresolved_review_action().is_some();
        let can_prepare = !self.operation_busy && !unresolved;
        let selected_anchor = self.selected_review_anchor.as_ref();
        let mut files = div().flex().flex_col().gap(px(6.0));
        for (file_index, file) in review.files.iter().enumerate() {
            let mut hunks = div().flex().flex_col().gap(px(4.0));
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                let (side, line) = if hunk.new_lines > 0 {
                    (DiffSide::New, hunk.new_start)
                } else {
                    (DiffSide::Old, hunk.old_start)
                };
                let anchor = ReviewAnchorSemantic {
                    file_id: file.id.clone(),
                    hunk_id: hunk.id.clone(),
                    side,
                    line,
                };
                let selected = selected_anchor == Some(&anchor);
                let select_entity = entity.clone();
                let anchor_for_click = anchor.clone();
                hunks = hunks.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.0))
                        .min_w(px(0.0))
                        .child(
                            Button::new(
                                ("review-anchor", file_index * 1024 + hunk_index),
                                t!("fleet.review.anchor", side = side.as_str(), line = line)
                                    .to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(if selected {
                                ButtonVariant::Default
                            } else {
                                ButtonVariant::Outline
                            })
                            .disabled(!can_prepare)
                            .on_click(move |_, _, cx| {
                                select_entity.update(cx, |this, cx| {
                                    this.selected_review_anchor = Some(anchor_for_click.clone());
                                    this.pending_review_preview = None;
                                    cx.notify();
                                });
                            }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .line_clamp(3)
                                .child(hunk.preview.clone()),
                        ),
                );
            }
            files = files.child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .p(px(8.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .mb(px(6.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(file.path.clone()),
                            )
                            .child(
                                Badge::new(
                                    t!(
                                        "fleet.review.file_state",
                                        change = file.change.as_str(),
                                        worktree = file.worktree.as_str(),
                                        conflict = file.conflict.as_str()
                                    )
                                    .to_string(),
                                )
                                .variant(BadgeVariant::Outline),
                            )
                            .child(
                                Badge::new(
                                    t!(
                                        "fleet.review.preview_metadata",
                                        kind = file.preview.kind.as_str(),
                                        bytes = file.preview.byte_size.map_or_else(
                                            || "—".to_owned(),
                                            |value| value.to_string()
                                        ),
                                        sanitized = if file.preview.sanitized {
                                            t!("fleet.review.sanitized_yes").to_string()
                                        } else {
                                            t!("fleet.review.sanitized_no").to_string()
                                        }
                                    )
                                    .to_string(),
                                )
                                .variant(BadgeVariant::Secondary),
                            ),
                    )
                    .child(hunks),
            );
        }

        let mut comments = div().flex().flex_col().gap(px(4.0));
        for comment in &review.comments {
            let selectable = matches!(
                comment.agent_state,
                CommentAgentState::NotSent | CommentAgentState::Refused
            ) && review.comment_is_batch_actionable(&comment.id);
            let selected = self.selected_review_comments.contains(&comment.id);
            let select_entity = entity.clone();
            let comment_id = comment.id.clone();
            comments = comments.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            t!(
                                "fleet.review.comment",
                                actor = comment.actor.as_str(),
                                file = comment.anchor.file_id.as_str(),
                                side = comment.anchor.side.as_str(),
                                line = comment.anchor.line,
                                revision = comment.revision.get(),
                                body = comment.body.as_str()
                            )
                            .to_string(),
                        ),
                    )
                    .child(
                        Button::new(
                            ElementId::from(SharedString::from(format!(
                                "select-review-comment-{}",
                                comment.id
                            ))),
                            if selected {
                                t!("fleet.review.comment_selected").to_string()
                            } else {
                                t!("fleet.review.comment_select").to_string()
                            },
                        )
                        .size(ButtonSize::Sm)
                        .variant(if selected {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .disabled(!can_prepare || !selectable)
                        .on_click(move |_, _, cx| {
                            select_entity.update(cx, |this, cx| {
                                if !this.selected_review_comments.remove(&comment_id) {
                                    this.selected_review_comments.insert(comment_id.clone());
                                }
                                this.pending_review_preview = None;
                                cx.notify();
                            });
                        }),
                    ),
            );
        }

        let mut attention = div().flex().flex_col().gap(px(4.0));
        for event in review.attention_events.iter().rev() {
            attention = attention.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(
                            "fleet.review.attention_event",
                            reason = event.reason.as_str(),
                            origin = event.origin_kind.as_str(),
                            revision = event.source_revision.get(),
                            unread = event.unread,
                            authority =
                                format!("{}:{}", event.authority.kind.as_str(), event.authority.id)
                        )
                        .to_string(),
                    ),
            );
        }

        let comment_entity = entity.clone();
        let approve_entity = entity.clone();
        let batch_entity = entity.clone();
        let can_comment = can_prepare
            && selected_anchor.is_some()
            && !self.review_comment_value.trim().is_empty();
        let selected_label = selected_anchor.map_or_else(
            || t!("fleet.review.anchor_none").to_string(),
            |anchor| {
                t!(
                    "fleet.review.anchor_selected",
                    file = anchor.file_id.as_str(),
                    hunk = anchor.hunk_id.as_str(),
                    side = anchor.side.as_str(),
                    line = anchor.line
                )
                .to_string()
            },
        );
        let mut effect_controls = div().flex().flex_wrap().gap(px(6.0)).child(
            Button::new(
                "prepare-review-comment-batch",
                t!(
                    "fleet.review.prepare_comment_batch",
                    count = self.selected_review_comments.len()
                )
                .to_string(),
            )
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Outline)
            .disabled(!can_prepare || self.selected_review_comments.is_empty())
            .on_click(move |_, _, cx| {
                batch_entity.update(cx, |this, cx| {
                    if this.prepare_review_comment_batch().is_err() {
                        this.review_receipt = Some((
                            t!("fleet.review.state_refused").to_string(),
                            t!("fleet.review.invalid_preview").to_string(),
                        ));
                    }
                    cx.notify();
                });
            }),
        );
        for proposal in review
            .proposals
            .iter()
            .filter(|proposal| review.proposal_is_actionable(&proposal.id))
        {
            let proposal_entity = entity.clone();
            let proposal_id = proposal.id.clone();
            effect_controls = effect_controls.child(
                Button::new(
                    ElementId::from(SharedString::from(format!(
                        "prepare-review-proposal-{}",
                        proposal.id
                    ))),
                    t!(
                        "fleet.review.prepare_proposal",
                        action = localized_review_proposal_kind(proposal.kind),
                        files = proposal.files.len()
                    )
                    .to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Outline)
                .disabled(!can_prepare)
                .on_click(move |_, _, cx| {
                    proposal_entity.update(cx, |this, cx| {
                        if this.prepare_review_proposal(&proposal_id).is_err() {
                            this.review_receipt = Some((
                                t!("fleet.review.state_refused").to_string(),
                                t!("fleet.review.invalid_preview").to_string(),
                            ));
                        }
                        cx.notify();
                    });
                }),
            );
        }
        for check in review
            .checks
            .iter()
            .filter(|check| review.check_is_rerunnable(&check.id))
        {
            let check_entity = entity.clone();
            let check_id = check.id.clone();
            effect_controls = effect_controls.child(
                Button::new(
                    ElementId::from(SharedString::from(format!(
                        "prepare-review-check-{}",
                        check.id
                    ))),
                    t!("fleet.review.prepare_check", check = check.id.as_str()).to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Outline)
                .disabled(!can_prepare)
                .on_click(move |_, _, cx| {
                    check_entity.update(cx, |this, cx| {
                        if this.prepare_review_check_rerun(&check_id).is_err() {
                            this.review_receipt = Some((
                                t!("fleet.review.state_refused").to_string(),
                                t!("fleet.review.invalid_preview").to_string(),
                            ));
                        }
                        cx.notify();
                    });
                }),
            );
        }
        if review.pull_request_is_mergeable() {
            let merge_entity = entity.clone();
            effect_controls = effect_controls.child(
                Button::new(
                    "prepare-review-merge",
                    t!("fleet.review.prepare_merge").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Outline)
                .disabled(!can_prepare)
                .on_click(move |_, _, cx| {
                    merge_entity.update(cx, |this, cx| {
                        if this.prepare_review_merge().is_err() {
                            this.review_receipt = Some((
                                t!("fleet.review.state_refused").to_string(),
                                t!("fleet.review.invalid_preview").to_string(),
                            ));
                        }
                        cx.notify();
                    });
                }),
            );
        }
        let controls = div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .p(px(10.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(
                            "fleet.review.exact_context",
                            workspace = review.workspace_id.as_str(),
                            revision = review.revision.get(),
                            anchor = selected_label
                        )
                        .to_string(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Input::new(&self.review_comment_state)
                            .size(InputSize::Sm)
                            .placeholder(t!("fleet.review.comment_placeholder").to_string())
                            .disabled(!can_prepare)
                            .on_change({
                                let entity = entity.clone();
                                move |value, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.review_comment_value = value.to_string();
                                        this.pending_review_preview = None;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(
                            "prepare-review-comment",
                            t!("fleet.review.prepare_comment").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(!can_comment)
                        .on_click(move |_, _, cx| {
                            comment_entity.update(cx, |this, cx| {
                                if this.prepare_review_comment().is_err() {
                                    this.review_receipt = Some((
                                        t!("fleet.review.state_refused").to_string(),
                                        t!("fleet.review.invalid_preview").to_string(),
                                    ));
                                }
                                cx.notify();
                            });
                        }),
                    )
                    .child(
                        Button::new(
                            "prepare-review-approval",
                            t!("fleet.review.prepare_approval").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(!can_prepare || !review.approval_is_actionable())
                        .on_click(move |_, _, cx| {
                            approve_entity.update(cx, |this, cx| {
                                if this.prepare_review_approval().is_err() {
                                    this.review_receipt = Some((
                                        t!("fleet.review.state_refused").to_string(),
                                        t!("fleet.review.invalid_preview").to_string(),
                                    ));
                                }
                                cx.notify();
                            });
                        }),
                    ),
            )
            .child(effect_controls)
            .child(self.render_review_confirmation(cx));

        div()
            .flex()
            .flex_col()
            .max_h(px(360.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(Self::render_review_badges(review))
            .child(
                div()
                    .id("platform-review-detail")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .p(px(10.0))
                    .flex()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(Self::review_column_title(
                                t!("fleet.review.files_title").to_string(),
                                review.files.len(),
                            ))
                            .child(files),
                    )
                    .child(
                        div()
                            .w(px(320.0))
                            .min_w(px(240.0))
                            .child(Self::review_column_title(
                                t!("fleet.review.comments_title").to_string(),
                                review.comments.len(),
                            ))
                            .child(comments)
                            .child(Self::review_column_title(
                                t!("fleet.review.attention_title").to_string(),
                                review.attention_events.len(),
                            ))
                            .child(attention),
                    ),
            )
            .child(controls)
            .into_any_element()
    }

    fn review_column_title(label: String, count: usize) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(6.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .child(label)
            .child(Badge::new(count.to_string()).variant(BadgeVariant::Secondary))
    }

    fn render_review_confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(preview) = self.pending_review_preview.as_ref() {
            let confirm_entity = cx.entity();
            let cancel_entity = confirm_entity.clone();
            let action = match preview.action() {
                ReviewAction::AddComment { anchor, body, .. } => t!(
                    "fleet.review.confirm_comment",
                    project = preview.target().project.as_str(),
                    workspace = preview.target().workspace.id(),
                    file = anchor.file_id().as_str(),
                    hunk = anchor.hunk_id().as_str(),
                    side = anchor.side().as_str(),
                    line = anchor.line(),
                    body = body.as_str(),
                    revision = preview.expected_revision().get()
                )
                .to_string(),
                ReviewAction::ApproveReview {
                    expected_review_revision,
                } => t!(
                    "fleet.review.confirm_approval",
                    project = preview.target().project.as_str(),
                    workspace = preview.target().workspace.id(),
                    review_revision = expected_review_revision.get(),
                    revision = preview.expected_revision().get()
                )
                .to_string(),
                _ => preview.confirmation_coordinates().map_or_else(
                    || t!("fleet.review.unsupported_action").to_string(),
                    |coordinates| {
                        let (action, coordinates) =
                            localized_external_review_coordinates(coordinates);
                        t!(
                            "fleet.review.confirm_external",
                            action = action,
                            project = preview.target().project.as_str(),
                            workspace = preview.target().workspace.id(),
                            revision = preview.expected_revision().get(),
                            coordinates = coordinates
                        )
                        .to_string()
                    },
                ),
            };
            return div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .p(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::warning())
                .rounded(px(6.0))
                .bg(ShellDeckColors::warning().opacity(0.12))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(11.0))
                        .child(action),
                )
                .child(
                    Button::new(
                        "confirm-review-action",
                        t!("fleet.review.confirm").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Default)
                    .on_click(move |_, _, cx| {
                        confirm_entity.update(cx, |this, cx| {
                            if let Some(preview) = this.dispatch_review_preview() {
                                cx.emit(FleetViewEvent::ExecuteReview(preview));
                            }
                            cx.notify();
                        });
                    }),
                )
                .child(
                    Button::new(
                        "cancel-review-action",
                        t!("fleet.review.cancel").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cancel_entity.update(cx, |this, cx| {
                            this.pending_review_preview = None;
                            cx.notify();
                        });
                    }),
                )
                .into_any_element();
        }
        if let Some((state, detail)) = self.review_receipt.as_ref() {
            return Alert::info()
                .title(t!("fleet.review.receipt", state = state.as_str()).to_string())
                .description(detail.clone())
                .into_any_element();
        }
        div().into_any_element()
    }

    fn render_action_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(preview) = self.pending_action.as_ref() else {
            return div();
        };
        let entity = cx.entity();
        let confirm_entity = entity.clone();
        let preview_for_event = preview.clone();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::warning())
            .bg(ShellDeckColors::warning().opacity(0.12))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(
                        t!(
                            "fleet.action.preview",
                            action = preview.action.as_str(),
                            target = preview.target.id.as_str(),
                            revision = preview
                                .expected_revision
                                .map_or_else(|| "?".to_string(), |value| value.to_string()),
                            parameter =
                                preview.parameter.as_ref().map_or("—", PlatformText::as_str)
                        )
                        .to_string(),
                    ),
            )
            .child(
                Button::new(
                    "confirm-platform-action",
                    t!("fleet.action.confirm").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Default)
                .disabled(self.operation_busy)
                .on_click(move |_, _, cx| {
                    confirm_entity.update(cx, |_this, cx| {
                        cx.emit(FleetViewEvent::Execute(preview_for_event.clone()));
                    });
                }),
            )
            .child(
                Button::new(
                    "cancel-platform-action",
                    t!("fleet.action.cancel").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .disabled(self.operation_busy)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.pending_action = None;
                        cx.notify();
                    });
                }),
            )
    }

    fn section_header(label: impl Into<SharedString>, count: usize) -> impl IntoElement {
        let label: SharedString = label.into();
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(14.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(label),
            )
            .child(Badge::new(count.to_string()).variant(BadgeVariant::Secondary))
    }

    fn render_resource(
        &self,
        resource: &ResourceRecord,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let freshness = resource.freshness.state.as_str();
        let freshness_color = match freshness {
            "fresh" => ShellDeckColors::success(),
            "stale" => ShellDeckColors::warning(),
            _ => ShellDeckColors::text_muted(),
        };
        let mut row = div()
            .flex()
            .items_start()
            .gap(px(9.0))
            .px(px(12.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.65))
            .child(
                div()
                    .mt(px(4.0))
                    .size(px(7.0))
                    .rounded_full()
                    .bg(freshness_color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(resource.summary.as_str().to_owned()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                Badge::new(resource.resource.kind.as_str().to_owned())
                                    .variant(BadgeVariant::Outline),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!(
                                        "{} · {} · rev {}",
                                        resource.resource.authority.as_str(),
                                        resource.resource.id.as_str(),
                                        resource.freshness.revision.get()
                                    )),
                            ),
                    ),
            );
        if resource.resource.kind == ResourceKind::Approval
            && resource.freshness.state.as_str() == "fresh"
            && resource.summary.as_str().starts_with("state=pending")
        {
            let entity = cx.entity();
            let approve_entity = entity.clone();
            let approve_target = resource.resource.clone();
            let deny_target = resource.resource.clone();
            let revision = resource.freshness.revision;
            row = row
                .child(
                    Button::new(
                        ElementId::from(SharedString::from(format!(
                            "approve-{}",
                            resource.resource.id.as_str()
                        ))),
                        t!("fleet.approval.grant").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Default)
                    .disabled(self.operation_busy || self.pending_action.is_some())
                    .on_click(move |_, _, cx| {
                        approve_entity.update(cx, |this, cx| {
                            this.pending_action = Some(PlatformActionPreview::new(
                                PlatformAction::DecideApproval,
                                approve_target.clone(),
                                Some(revision),
                                PlatformText::new("grant").ok(),
                            ));
                            cx.notify();
                        });
                    }),
                )
                .child(
                    Button::new(
                        ElementId::from(SharedString::from(format!(
                            "deny-{}",
                            resource.resource.id.as_str()
                        ))),
                        t!("fleet.approval.deny").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Outline)
                    .disabled(self.operation_busy || self.pending_action.is_some())
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.pending_action = Some(PlatformActionPreview::new(
                                PlatformAction::DecideApproval,
                                deny_target.clone(),
                                Some(revision),
                                PlatformText::new("deny").ok(),
                            ));
                            cx.notify();
                        });
                    }),
                );
        }
        row
    }

    fn render_resources(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let resources = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.resources.as_slice())
            .unwrap_or_default();
        let mut list = div()
            .id("platform-resources")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if resources.is_empty() {
            list = list.child(
                div()
                    .p(px(16.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("fleet.resources.empty").to_string()),
            );
        } else {
            for resource in resources {
                list = list.child(self.render_resource(resource, cx));
            }
        }
        if let Some(snapshot) = self.snapshot.as_ref() {
            let receipts = snapshot.view.receipts().collect::<Vec<_>>();
            if !receipts.is_empty() {
                list = list.child(Self::section_header(
                    t!("fleet.receipts.section").to_string(),
                    receipts.len(),
                ));
                for receipt in receipts {
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border().opacity(0.65))
                            .child(Badge::new(receipt.outcome.as_str().to_owned()).variant(
                                match receipt.outcome {
                                    shelldeck_core::config::platform::ReceiptOutcome::Completed => {
                                        BadgeVariant::Default
                                    }
                                    shelldeck_core::config::platform::ReceiptOutcome::Accepted => {
                                        BadgeVariant::Warning
                                    }
                                    _ => BadgeVariant::Destructive,
                                },
                            ))
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!(
                                        "{} · {} · {}",
                                        receipt.action.as_str(),
                                        receipt.target.id.as_str(),
                                        receipt.id.as_str()
                                    )),
                            ),
                    );
                }
            }
        }
        list
    }

    fn render_session(&self, session: &SessionRecord, cx: &mut Context<Self>) -> impl IntoElement {
        let coordinate = &session.session.resource;
        let key = resource_key(coordinate);
        let pane = self.cockpit.pane(coordinate);
        let attached = pane.is_some();
        let lease = pane.and_then(|pane| pane.lease.as_ref());
        let selected = self.selected_session.as_deref() == Some(coordinate.id.as_str());
        let entity = cx.entity();
        let select_id = coordinate.id.as_str().to_owned();
        let session_coordinate = coordinate.clone();
        let observe_coordinate = coordinate.clone();
        let observe_entity = entity.clone();
        let control_coordinate = coordinate.clone();
        let control_entity = entity.clone();
        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "platform-session-{}",
                coordinate.id.as_str()
            ))))
            .w_full()
            .flex()
            .items_start()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(11.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.65))
            .when(selected, |row| {
                row.bg(ShellDeckColors::primary().opacity(0.08))
            })
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    if attached {
                        this.select_session(&session_coordinate, cx);
                    } else {
                        this.selected_session = Some(select_id.clone());
                    }
                    cx.notify();
                });
            })
            .child(lucide_icon(
                "messages-square",
                16.0,
                if attached {
                    ShellDeckColors::success()
                } else {
                    ShellDeckColors::text_muted()
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(session.session.summary.as_str().to_owned()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                Badge::new(if attached {
                                    t!("fleet.session.attached").to_string()
                                } else {
                                    t!("fleet.session.observed").to_string()
                                })
                                .variant(if attached {
                                    BadgeVariant::Default
                                } else {
                                    BadgeVariant::Outline
                                }),
                            )
                            .children(lease.map(|lease| {
                                Badge::new(
                                    t!(
                                        "fleet.session.control",
                                        expiry = lease.expires_at.as_millis()
                                    )
                                    .to_string(),
                                )
                                .variant(BadgeVariant::Warning)
                            }))
                            .children(pane.and_then(|pane| {
                                (pane.unread > 0).then(|| {
                                    Badge::new(
                                        t!("fleet.session.unread", count = pane.unread).to_string(),
                                    )
                                    .variant(BadgeVariant::Default)
                                })
                            }))
                            .children(pane.and_then(|pane| {
                                pane.control_lost.then(|| {
                                    Badge::new(t!("fleet.session.control_lost").to_string())
                                        .variant(BadgeVariant::Destructive)
                                })
                            }))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(coordinate.id.as_str().to_owned()),
                            ),
                    ),
            );

        if session.attachable {
            row = row.child(
                Button::new(
                    ElementId::from(SharedString::from(format!("observe-{key}"))),
                    if attached {
                        t!("fleet.session.detach").to_string()
                    } else {
                        t!("fleet.session.attach").to_string()
                    },
                )
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .disabled(self.operation_busy)
                .on_click(move |_, _, cx| {
                    observe_entity.update(cx, |_this, cx| {
                        if attached {
                            cx.emit(FleetViewEvent::Detach(observe_coordinate.clone()));
                        } else {
                            cx.emit(FleetViewEvent::Attach(observe_coordinate.clone()));
                        }
                    });
                }),
            );
        }
        if session.controllable && attached {
            let lease_for_event = lease.cloned();
            row = row.child(
                Button::new(
                    ElementId::from(SharedString::from(format!("control-{key}"))),
                    if lease.is_some() {
                        t!("fleet.session.release").to_string()
                    } else {
                        t!("fleet.session.claim").to_string()
                    },
                )
                .variant(if lease.is_some() {
                    ButtonVariant::Outline
                } else {
                    ButtonVariant::Default
                })
                .size(ButtonSize::Sm)
                .disabled(self.operation_busy)
                .on_click(move |_, _, cx| {
                    control_entity.update(cx, |_this, cx| {
                        if let Some(lease) = lease_for_event.clone() {
                            cx.emit(FleetViewEvent::ReleaseControl(
                                control_coordinate.clone(),
                                lease,
                            ));
                        } else {
                            cx.emit(FleetViewEvent::ClaimControl(control_coordinate.clone()));
                        }
                    });
                }),
            );
        }
        row
    }

    fn render_sessions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_query.trim().to_ascii_lowercase();
        let sessions = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .sessions
                    .iter()
                    .filter(|session| {
                        query.is_empty()
                            || session
                                .session
                                .resource
                                .id
                                .as_str()
                                .to_ascii_lowercase()
                                .contains(&query)
                            || session
                                .session
                                .summary
                                .as_str()
                                .to_ascii_lowercase()
                                .contains(&query)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut list = div()
            .id("platform-sessions")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if sessions.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .p(px(24.0))
                    .child(lucide_icon(
                        "messages-square",
                        24.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("fleet.sessions.empty").to_string()),
                    ),
            );
        } else {
            for session in sessions {
                list = list.child(self.render_session(session, cx));
            }
        }
        list
    }

    fn render_session_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.search_state)
            .size(InputSize::Sm)
            .placeholder(t!("fleet.sessions.search").to_string())
            .clearable(true)
            .prefix(
                svg()
                    .path("icons/lucide/search.svg")
                    .size(px(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
            .on_change({
                let entity = cx.entity();
                move |value, cx| {
                    entity.update(cx, |this, cx| {
                        this.search_query = value.to_string();
                        cx.notify();
                    });
                }
            })
    }

    fn render_pane_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let mut tabs = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .id("platform-pane-tabs")
            .overflow_x_scroll();
        if self.cockpit.panes().len() == 0 {
            return tabs.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("fleet.panes.empty").to_string()),
            );
        }
        for pane in self.cockpit.panes() {
            let coordinate = pane.attachment.session.clone();
            let label = coordinate.id.as_str().to_owned();
            let selected = self
                .cockpit
                .selected()
                .is_some_and(|selected| selected.attachment.session == coordinate);
            let tab_entity = entity.clone();
            let mut tab = Button::new(
                ElementId::from(SharedString::from(format!("pane-{label}"))),
                if pane.unread > 0 {
                    format!("{label} ({})", pane.unread)
                } else {
                    label
                },
            )
            .size(ButtonSize::Sm)
            .variant(if selected {
                ButtonVariant::Default
            } else {
                ButtonVariant::Outline
            });
            tab = tab.on_click(move |_, _, cx| {
                tab_entity.update(cx, |this, cx| {
                    this.select_session(&coordinate, cx);
                    cx.notify();
                });
            });
            tabs = tabs.child(tab);
        }
        tabs
    }

    fn render_history_event(event: &SessionHistoryEvent) -> AnyElement {
        let (title, detail, evidence, warning) = match event {
            SessionHistoryEvent::Message {
                evidence,
                role,
                text,
                truncated,
                ..
            } => (
                match role {
                    shelldeck_core::config::platform::SessionHistoryRole::Assistant => {
                        t!("fleet.history.role.assistant").to_string()
                    }
                    shelldeck_core::config::platform::SessionHistoryRole::User => {
                        t!("fleet.history.role.user").to_string()
                    }
                },
                text.as_str().to_owned(),
                Some(match evidence {
                    shelldeck_core::config::platform::SessionHistoryEvidence::Authoritative => {
                        t!("fleet.history.evidence.authoritative").to_string()
                    }
                    shelldeck_core::config::platform::SessionHistoryEvidence::Synthetic => {
                        t!("fleet.history.evidence.synthetic").to_string()
                    }
                }),
                *truncated,
            ),
            SessionHistoryEvent::ToolState {
                evidence,
                state,
                label,
                truncated,
                ..
            } => (
                t!("fleet.history.tool").to_string(),
                t!(
                    "fleet.history.tool_state",
                    state = match state {
                        shelldeck_core::config::platform::SessionHistoryToolState::Pending => t!("fleet.history.state.pending"),
                        shelldeck_core::config::platform::SessionHistoryToolState::InProgress => t!("fleet.history.state.in_progress"),
                        shelldeck_core::config::platform::SessionHistoryToolState::Completed => t!("fleet.history.state.completed"),
                        shelldeck_core::config::platform::SessionHistoryToolState::Error => t!("fleet.history.state.error"),
                    },
                    label = label.as_ref().map_or("—", |label| label.as_str())
                )
                .to_string(),
                Some(match evidence {
                    shelldeck_core::config::platform::SessionHistoryEvidence::Authoritative => t!("fleet.history.evidence.authoritative").to_string(),
                    shelldeck_core::config::platform::SessionHistoryEvidence::Synthetic => t!("fleet.history.evidence.synthetic").to_string(),
                }),
                *truncated,
            ),
            SessionHistoryEvent::RunState { state, .. } => (
                t!("fleet.history.run").to_string(),
                match state {
                    shelldeck_core::config::platform::SessionHistoryRunState::Started => t!("fleet.history.run_state.started"),
                    shelldeck_core::config::platform::SessionHistoryRunState::CancelRequested => t!("fleet.history.run_state.cancel_requested"),
                    shelldeck_core::config::platform::SessionHistoryRunState::Completed => t!("fleet.history.run_state.completed"),
                    shelldeck_core::config::platform::SessionHistoryRunState::Failed => t!("fleet.history.run_state.failed"),
                    shelldeck_core::config::platform::SessionHistoryRunState::Cancelled => t!("fleet.history.run_state.cancelled"),
                    shelldeck_core::config::platform::SessionHistoryRunState::TimedOut => t!("fleet.history.run_state.timed_out"),
                }
                .to_string(),
                None,
                false,
            ),
            SessionHistoryEvent::Unknown { source, .. } => (
                t!("fleet.history.unknown_title").to_string(),
                match source {
                    shelldeck_core::config::platform::SessionHistoryUnknownSource::AdapterEvent => t!("fleet.history.unknown.adapter"),
                    shelldeck_core::config::platform::SessionHistoryUnknownSource::SimulationEvent => t!("fleet.history.unknown.simulation"),
                }
                .to_string(),
                None,
                true,
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if warning {
                ShellDeckColors::warning()
            } else {
                ShellDeckColors::border()
            })
            .bg(ShellDeckColors::bg_surface())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(title),
                    )
                    .children(
                        evidence
                            .map(|evidence| Badge::new(evidence).variant(BadgeVariant::Outline)),
                    )
                    .children(warning.then(|| {
                        Badge::new(t!("fleet.history.truncated").to_string())
                            .variant(BadgeVariant::Warning)
                    })),
            )
            .child(
                div()
                    .min_w(px(0.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_selected_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(pane) = self.cockpit.selected() else {
            return div().p(px(12.0));
        };
        let session = &pane.attachment.session;
        if self.selected_session.as_deref() != Some(session.id.as_str()) {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!("fleet.pane.attach_to_read").to_string());
        }
        let stream_label = match pane.stream {
            PaneStreamState::Live => t!("fleet.pane.live").to_string(),
            PaneStreamState::Resynchronized => t!("fleet.pane.resynchronized").to_string(),
            PaneStreamState::Offline => t!("fleet.pane.offline").to_string(),
            PaneStreamState::Error => t!("fleet.pane.error").to_string(),
        };
        let stream_variant = match pane.stream {
            PaneStreamState::Live => BadgeVariant::Default,
            PaneStreamState::Resynchronized => BadgeVariant::Warning,
            PaneStreamState::Offline | PaneStreamState::Error => BadgeVariant::Destructive,
        };
        let session_record = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .sessions
                .iter()
                .find(|record| record.session.resource == *session)
        });
        let run = session_record.and_then(|record| record.run.as_ref());
        let entity = cx.entity();
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(Badge::new(stream_label).variant(stream_variant))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · cursor {}",
                                session.id.as_str(),
                                pane.attachment.cursor.sequence.get()
                            )),
                    ),
            );
        if let Some(state) = pane.command_state.as_ref() {
            let freshness = match state.session.freshness.state {
                shelldeck_core::config::platform::FreshnessState::Fresh => {
                    t!("fleet.command.fresh").to_string()
                }
                shelldeck_core::config::platform::FreshnessState::Stale => {
                    t!("fleet.command.stale").to_string()
                }
                shelldeck_core::config::platform::FreshnessState::Unknown => {
                    t!("fleet.command.unknown").to_string()
                }
            };
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Badge::new(
                            t!(
                                "fleet.command.revision",
                                revision = state.session.freshness.revision.get()
                            )
                            .to_string(),
                        )
                        .variant(BadgeVariant::Outline),
                    )
                    .child(Badge::new(freshness).variant(BadgeVariant::Secondary))
                    .child(
                        Badge::new(
                            t!(
                                "fleet.command.approvals",
                                count = state.pending_approvals.len()
                            )
                            .to_string(),
                        )
                        .variant(if state.pending_approvals.is_empty() {
                            BadgeVariant::Outline
                        } else {
                            BadgeVariant::Warning
                        }),
                    ),
            );
        }
        if pane.mutation_fence.is_some() {
            content = content.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!("fleet.command.awaiting_revision").to_string()),
            );
        } else if pane.pending_follow_up.is_some() {
            content = content.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::warning())
                    .child(t!("fleet.command.reconciling").to_string()),
            );
        }
        if let Some(lease) = pane.lease.as_ref() {
            content = content.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::warning())
                    .child(
                        t!(
                            "fleet.pane.controller_self",
                            expiry = lease.expires_at.as_millis()
                        )
                        .to_string(),
                    ),
            );
        } else if pane.control_lost {
            content = content.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::error())
                    .child(t!("fleet.pane.controller_lost").to_string()),
            );
        }
        if let Some(run) = run {
            let revision = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.view.resource(run))
                .map(|resource| resource.freshness.revision);
            if self.pending_action.is_none() && pane.lease.is_some() {
                let target = run.clone();
                content = content.child(
                    Button::new("preview-stop-run", t!("fleet.action.stop_run").to_string())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(self.operation_busy)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.pending_action = Some(PlatformActionPreview::new(
                                    PlatformAction::StopRun,
                                    target.clone(),
                                    revision,
                                    None,
                                ));
                                cx.notify();
                            });
                        }),
                );
            }
        }
        if let Some(snapshot) = self.snapshot.as_ref() {
            let receipts = snapshot
                .view
                .receipts()
                .filter(|receipt| {
                    receipt.target == *session || run.is_some_and(|run| receipt.target == *run)
                })
                .collect::<Vec<_>>();
            if !receipts.is_empty() {
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(t!("fleet.receipts.section").to_string()),
                        )
                        .children(receipts.into_iter().map(|receipt| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    Badge::new(receipt.outcome.as_str().to_owned()).variant(
                                        match receipt.outcome {
                                            shelldeck_core::config::platform::ReceiptOutcome::Completed => {
                                                BadgeVariant::Default
                                            }
                                            shelldeck_core::config::platform::ReceiptOutcome::Accepted => {
                                                BadgeVariant::Warning
                                            }
                                            _ => BadgeVariant::Destructive,
                                        },
                                    ),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(format!(
                                            "{} · {} · rev {}",
                                            receipt.action.as_str(),
                                            receipt.id.as_str(),
                                            receipt.revision.get()
                                        )),
                                )
                        })),
                );
            }
        }
        let mut transcript = div()
            .id("platform-retained-transcript")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0));
        if pane.retained_events.is_empty() {
            transcript = transcript.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("fleet.history.empty").to_string()),
            );
        } else {
            transcript =
                transcript.children(pane.retained_events.iter().map(Self::render_history_event));
        }
        if pane.retained_resynchronized {
            transcript = transcript.child(
                Alert::warning()
                    .title(t!("fleet.history.resynchronized").to_string())
                    .description(t!("fleet.history.resynchronized_detail").to_string()),
            );
        }
        if let Some((outcome, explanation)) = pane.retained_refusal.as_ref() {
            transcript = transcript.child(
                Alert::error()
                    .title(t!("fleet.action.refused", outcome = outcome.as_str()).to_string())
                    .description(explanation.as_str().to_owned()),
            );
        }
        let can_send = pane.lease.is_some()
            && pane.command_state.as_ref().is_some_and(|state| {
                state.session.freshness.state
                    == shelldeck_core::config::platform::FreshnessState::Fresh
            })
            && pane.mutation_fence.is_none()
            && pane.pending_follow_up.is_none()
            && !self.composer_value.trim().is_empty()
            && !self.operation_busy;
        let send_entity = cx.entity();
        let composer = div()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .p(px(12.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("fleet.composer.authority_notice").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Input::new(&self.composer_state)
                            .size(InputSize::Md)
                            .placeholder(t!("fleet.composer.placeholder").to_string())
                            .disabled(pane.lease.is_none())
                            .on_change({
                                let entity = cx.entity();
                                move |value, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.composer_value = value.to_string();
                                        if let Some(session) = this.selected_session.as_ref() {
                                            this.drafts.insert(session.clone(), value.to_string());
                                        }
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("platform-follow-up", t!("fleet.composer.send").to_string())
                            .variant(ButtonVariant::Default)
                            .size(ButtonSize::Md)
                            .disabled(!can_send)
                            .loading(self.operation_busy)
                            .on_click(move |_, _, cx| {
                                send_entity.update(cx, |this, cx| {
                                    if this.operation_busy {
                                        return;
                                    }
                                    match this.prepare_selected_follow_up() {
                                        Ok(follow_up) => {
                                            this.operation_busy = true;
                                            cx.emit(FleetViewEvent::FollowUp(follow_up));
                                        }
                                        Err(error) => this.set_operation_error(error.to_string()),
                                    }
                                    cx.notify();
                                });
                            }),
                    ),
            );
        div()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(content)
            .child(transcript)
            .child(composer)
    }
}

fn semantic_words(key: &str) -> String {
    key.split('.')
        .skip(1)
        .map(|part| part.replace('_', " "))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn resource_key(resource: &ResourceCoordinate) -> String {
    format!(
        "{}:{}:{}",
        resource.authority.as_str(),
        resource.kind.as_str(),
        resource.id.as_str()
    )
}

impl Render for FleetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let resource_count = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.resources.len());
        let session_count = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.sessions.len());
        let mut content = div()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .flex()
            .child(
                div()
                    .w(px(380.0))
                    .min_w(px(300.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(ShellDeckColors::border())
                    .bg(ShellDeckColors::bg_sidebar())
                    .child(Self::section_header(
                        t!("fleet.resources.section").to_string(),
                        resource_count,
                    ))
                    .child(self.render_resources(cx)),
            )
            .child(
                div()
                    .w(px(340.0))
                    .min_w(px(280.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(ShellDeckColors::border())
                    .child(Self::section_header(
                        t!("fleet.sessions.section").to_string(),
                        session_count,
                    ))
                    .child(
                        div()
                            .px(px(10.0))
                            .py(px(8.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(self.render_session_search(cx)),
                    )
                    .child(self.render_sessions(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.render_pane_tabs(cx))
                    .child(self.render_selected_pane(cx)),
            );
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .absolute()
                    .left(px(16.0))
                    .right(px(16.0))
                    .bottom(px(16.0))
                    .child(
                        Alert::error()
                            .title(t!("fleet.error.title").to_string())
                            .description(error.clone()),
                    ),
            );
        }
        if let Some((outcome, explanation)) = &self.refusal {
            content = content.child(
                div()
                    .absolute()
                    .left(px(16.0))
                    .right(px(16.0))
                    .bottom(px(16.0))
                    .child(
                        Alert::error()
                            .title(t!("fleet.action.refused", outcome = outcome).to_string())
                            .description(explanation.clone()),
                    ),
            );
        }
        div()
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(self.render_action_preview(cx))
            .child(self.render_review_summary(cx))
            .child(content)
    }
}

#[cfg(test)]
mod review_render_tests {
    use super::{
        exact_review_target_index, review_dispatch_directive, same_exact_review_snapshot,
        semantic_words, ReviewDispatchDirective,
    };
    use shelldeck_core::config::platform_review::PlatformReviewTarget;
    use shelldeck_core::config::workspace_catalog::{
        PlatformContextRef, PlatformMappingReconciliation, PlatformV2Mapping,
    };

    fn target(project: &str, workspace: &str) -> PlatformReviewTarget {
        PlatformReviewTarget::from_exact_mapping(&PlatformV2Mapping {
            reconciliation_revision: 1,
            project: PlatformContextRef {
                id: project.to_owned(),
                revision: 1,
            },
            checkout: PlatformContextRef {
                id: "checkout-1".to_owned(),
                revision: 1,
            },
            user_workspace: PlatformContextRef {
                id: workspace.to_owned(),
                revision: 1,
            },
            reconciliation: PlatformMappingReconciliation::Exact {
                reconciled_at_millis: 1,
            },
        })
        .unwrap()
    }

    // SDTEST-1779
    #[test]
    fn canonical_semantic_keys_drive_the_actual_review_badge_words() {
        let cases = [
            ("attention.idle", "idle"),
            ("attention.needs_you", "needs you"),
            ("attention.working", "working"),
            ("attention.blocked", "blocked"),
            ("attention.done", "done"),
            ("attention_reason.review_requested", "review requested"),
            ("attention_reason.check_running", "check running"),
            ("attention_reason.external_blocker", "external blocker"),
            ("attention_reason.complete", "complete"),
            ("review.dismissed", "dismissed"),
            ("review.pending", "pending"),
            ("review.changes_requested", "changes requested"),
            ("review.approved", "approved"),
            ("check.cancelled.optional", "cancelled · optional"),
            ("check.unavailable.required", "unavailable · required"),
            ("check.failed.required", "failed · required"),
            ("check.queued.optional", "queued · optional"),
            ("check.running.required", "running · required"),
            ("check.passed.required", "passed · required"),
            ("pull_request.absent.unknown", "absent · unknown"),
            ("pull_request.open.blocked", "open · blocked"),
            ("pull_request.draft.stale", "draft · stale"),
            ("pull_request.closed.blocked", "closed · blocked"),
            ("pull_request.merged.ready", "merged · ready"),
            ("delivery.not_delivered", "not delivered"),
            ("delivery.pending", "pending"),
            ("delivery.failed", "failed"),
            ("delivery.delivered", "delivered"),
            ("preview.none.raw", "none · raw"),
            ("preview.text.sanitized", "text · sanitized"),
            ("preview.image.sanitized", "image · sanitized"),
            ("preview.binary.raw", "binary · raw"),
            ("preview.html.sanitized", "html · sanitized"),
            ("freshness.unknown", "unknown"),
            ("freshness.stale", "stale"),
            ("freshness.fresh", "fresh"),
        ];

        for (semantic_key, visible_words) in cases {
            assert_eq!(
                semantic_words(semantic_key),
                visible_words,
                "{semantic_key}"
            );
        }
    }

    // SDTEST-1783
    #[test]
    fn review_dispatch_reducer_switches_permanently_from_execute_to_lookup() {
        assert_eq!(
            review_dispatch_directive(false, false),
            ReviewDispatchDirective::Idle
        );
        assert_eq!(
            review_dispatch_directive(true, false),
            ReviewDispatchDirective::ExecuteOnce
        );
        assert_eq!(
            review_dispatch_directive(false, true),
            ReviewDispatchDirective::LookupOnly
        );
        assert_eq!(
            review_dispatch_directive(true, true),
            ReviewDispatchDirective::LookupOnly,
            "an unresolved idempotency key must dominate any later draft"
        );
    }

    // SDTEST-1785
    #[test]
    fn unresolved_review_keys_remain_bound_to_their_exact_workspace() {
        let first = target("project-1", "wc_user_1");
        let second = target("project-1", "wc_user_2");
        let actions = vec![first.clone()];

        assert_eq!(
            exact_review_target_index(&actions, Some(&first), |target| target),
            Some(0)
        );
        assert!(exact_review_target_index(&actions, Some(&second), |target| target).is_none());
        assert!(exact_review_target_index(&actions, None, |target| target).is_none());
    }

    // SDTEST-1792
    #[test]
    fn review_selections_survive_only_the_same_exact_target_and_revision() {
        let first = target("project-1", "wc_user_1");
        let second = target("project-1", "wc_user_2");

        assert!(same_exact_review_snapshot(
            Some(&first),
            Some(41),
            Some(&first),
            Some(41)
        ));
        assert!(!same_exact_review_snapshot(
            Some(&first),
            Some(41),
            Some(&first),
            Some(42)
        ));
        assert!(!same_exact_review_snapshot(
            Some(&first),
            Some(41),
            Some(&second),
            Some(41)
        ));
        assert!(!same_exact_review_snapshot(None, None, None, None));
    }
}
