//! Typed, client-only access to the shared Automonique platform contract.
//!
//! ShellDeck renders platform state and may request explicit attach/control
//! operations. It does not define wire types, execute provider jobs, or own a
//! runtime loop.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use automonique_platform_client::platform_v2_client::{
    NegotiationResult, PlatformV2Client, PlatformV2ClientError, ReviewReadResult,
};
pub use automonique_platform_client::{ActionResult, ControlClaimResult};
use automonique_platform_client::{
    BearerToken, HttpsTransport, PlatformClient, PlatformView, SessionCommandStateResult,
    SessionHistoryResult, SessionListResult, SubscriptionApply, SubscriptionResult,
};
pub use automonique_protocol::platform::{
    ActionReceipt, Attachment, Capabilities, ClientId, ControlLease, ControlLeaseId,
    ExecuteRequest, FreshnessState, GetReceiptRequest, IdempotencyKey, PlatformAction,
    PlatformCursor, PlatformMethod, PlatformParameter, PlatformText, PlatformTransport, ReceiptId,
    ReceiptOutcome, ReleaseControlRequest, ResourceAuthority, ResourceCoordinate, ResourceKind,
    ResourceRecord, SessionCommandState, SessionFollowUpRequest, SessionHistoryEvent,
    SessionHistoryEvidence, SessionHistoryPage, SessionHistoryRole, SessionHistoryRunState,
    SessionHistoryToolState, SessionHistoryUnknownSource, SessionRecord,
};
use automonique_protocol::platform_v2::PlatformVersionOffer;
pub use automonique_protocol::primitives::Revision;
use url::Url;

use super::platform_review::{
    PlatformReviewLoad, PlatformReviewSemantic, PlatformReviewTarget, PlatformReviewUnavailable,
};
use crate::error::{Result, ShellDeckError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformSnapshot {
    pub capabilities: Capabilities,
    pub resources: Vec<ResourceRecord>,
    pub cursor: PlatformCursor,
    pub sessions: Vec<SessionRecord>,
    pub sessions_cursor: PlatformCursor,
    pub view: PlatformView,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformRefresh {
    pub snapshot: PlatformSnapshot,
    pub attachments: Vec<AttachmentRefresh>,
    pub events: usize,
    pub resynchronized: bool,
}

const RETAINED_HISTORY_PAGE_LIMIT: u16 = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedSessionRead {
    pub session: ResourceCoordinate,
    pub after: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetainedHistoryUpdate {
    Replace {
        page: SessionHistoryPage,
        retention_gap: bool,
    },
    Append(SessionHistoryPage),
    Refused {
        outcome: ReceiptOutcome,
        explanation: PlatformText,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedSessionUpdate {
    pub session: ResourceCoordinate,
    pub history: RetainedHistoryUpdate,
    pub command_state: SessionCommandStateResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformFollowUp {
    pub request: SessionFollowUpRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformFollowUpResult {
    Receipt {
        follow_up: PlatformFollowUp,
        receipt: ActionReceipt,
    },
    Refused {
        follow_up: PlatformFollowUp,
        outcome: ReceiptOutcome,
        explanation: PlatformText,
    },
    ReconciliationPending {
        follow_up: PlatformFollowUp,
        category: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentRefresh {
    Updated {
        attachment: Attachment,
        events: usize,
    },
    Resynchronized(Attachment),
    Failed {
        attachment: Attachment,
        category: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformActionPreview {
    pub action: PlatformAction,
    pub target: ResourceCoordinate,
    pub expected_revision: Option<automonique_protocol::primitives::Revision>,
    pub parameter: Option<PlatformText>,
    idempotency_key: IdempotencyKey,
}

impl PlatformActionPreview {
    #[must_use]
    pub fn new(
        action: PlatformAction,
        target: ResourceCoordinate,
        expected_revision: Option<automonique_protocol::primitives::Revision>,
        parameter: Option<PlatformText>,
    ) -> Self {
        Self {
            action,
            target,
            expected_revision,
            parameter,
            idempotency_key: unique_key(action.as_str()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneStreamState {
    Live,
    Resynchronized,
    Offline,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformPaneState {
    pub attachment: Attachment,
    pub lease: Option<ControlLease>,
    pub unread: usize,
    pub stream: PaneStreamState,
    pub error_category: Option<String>,
    pub control_lost: bool,
    pub retained_events: Vec<SessionHistoryEvent>,
    pub retained_cursor: Option<u64>,
    pub retained_has_more: bool,
    pub retained_resynchronized: bool,
    pub command_state: Option<SessionCommandState>,
    pub mutation_fence: Option<Revision>,
    pub pending_follow_up: Option<PlatformFollowUp>,
    pub retained_refusal: Option<(ReceiptOutcome, PlatformText)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformCockpitState {
    panes: BTreeMap<String, PlatformPaneState>,
    selected: Option<String>,
    online: bool,
}

impl Default for PlatformCockpitState {
    fn default() -> Self {
        Self {
            panes: BTreeMap::new(),
            selected: None,
            online: true,
        }
    }
}

impl PlatformCockpitState {
    pub fn attach(&mut self, attachment: Attachment) {
        let key = resource_key(&attachment.session);
        self.panes.insert(
            key.clone(),
            PlatformPaneState {
                attachment,
                lease: None,
                unread: 0,
                stream: PaneStreamState::Live,
                error_category: None,
                control_lost: false,
                retained_events: Vec::new(),
                retained_cursor: None,
                retained_has_more: false,
                retained_resynchronized: false,
                command_state: None,
                mutation_fence: None,
                pending_follow_up: None,
                retained_refusal: None,
            },
        );
        self.selected.get_or_insert(key);
    }

    pub fn detach(&mut self, session: &ResourceCoordinate) -> Option<Attachment> {
        let key = resource_key(session);
        let removed = self.panes.remove(&key).map(|pane| pane.attachment);
        if self.selected.as_deref() == Some(key.as_str()) {
            self.selected = self.panes.keys().next().cloned();
            if let Some(selected) = self.selected.as_ref() {
                if let Some(pane) = self.panes.get_mut(selected) {
                    pane.unread = 0;
                }
            }
        }
        removed
    }

    pub fn select(&mut self, session: &ResourceCoordinate) -> bool {
        let key = resource_key(session);
        let Some(pane) = self.panes.get_mut(&key) else {
            return false;
        };
        pane.unread = 0;
        self.selected = Some(key);
        true
    }

    pub fn selected(&self) -> Option<&PlatformPaneState> {
        self.selected.as_ref().and_then(|key| self.panes.get(key))
    }

    pub fn pane(&self, session: &ResourceCoordinate) -> Option<&PlatformPaneState> {
        self.panes.get(&resource_key(session))
    }

    pub fn panes(&self) -> impl ExactSizeIterator<Item = &PlatformPaneState> {
        self.panes.values()
    }

    pub fn retain_directory_sessions(&mut self, sessions: &[SessionRecord]) {
        self.panes.retain(|_, pane| {
            sessions
                .iter()
                .any(|record| record.session.resource == pane.attachment.session)
        });
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !self.panes.contains_key(selected))
        {
            self.selected = self.panes.keys().next().cloned();
        }
    }

    pub fn retained_reads(&self) -> Vec<RetainedSessionRead> {
        self.panes
            .values()
            .map(|pane| RetainedSessionRead {
                session: pane.attachment.session.clone(),
                after: pane.retained_cursor,
            })
            .collect()
    }

    pub fn pending_follow_ups(&self) -> Vec<PlatformFollowUp> {
        self.panes
            .values()
            .filter_map(|pane| pane.pending_follow_up.clone())
            .collect()
    }

    pub fn prepare_follow_up(
        &mut self,
        session: &ResourceCoordinate,
        text: impl Into<String>,
    ) -> Result<PlatformFollowUp> {
        if !self.online {
            return Err(ShellDeckError::Connection(
                "retained session is offline".to_string(),
            ));
        }
        let pane = self
            .panes
            .get_mut(&resource_key(session))
            .filter(|pane| pane.attachment.session == *session)
            .ok_or_else(|| ShellDeckError::Connection("session is not attached".to_string()))?;
        let lease = pane
            .lease
            .as_ref()
            .filter(|lease| {
                lease.session == *session
                    && lease.client == pane.attachment.client
                    && lease.expires_at.as_millis() > unix_millis()
            })
            .ok_or_else(|| {
                ShellDeckError::Connection("session control is not authorized".to_string())
            })?;
        if pane.pending_follow_up.is_some() || pane.mutation_fence.is_some() {
            return Err(ShellDeckError::Connection(
                "session command is awaiting reconciliation or a newer revision".to_string(),
            ));
        }
        let state = pane
            .command_state
            .as_ref()
            .filter(|state| {
                state.session.resource == *session
                    && state.session.freshness.state == FreshnessState::Fresh
            })
            .ok_or_else(|| {
                ShellDeckError::Connection("session command state is not fresh".to_string())
            })?;
        let request = SessionFollowUpRequest {
            client: lease.client.clone(),
            session: session.clone(),
            expected_session_revision: state.session.freshness.revision,
            idempotency_key: unique_key("follow-up"),
            text: PlatformParameter::new(text.into())
                .map_err(|_| ShellDeckError::Connection("follow-up text is invalid".to_string()))?,
        };
        let follow_up = PlatformFollowUp { request };
        pane.pending_follow_up = Some(follow_up.clone());
        Ok(follow_up)
    }

    pub fn apply_retained_update(&mut self, update: RetainedSessionUpdate) -> bool {
        if matches!(
            &update.command_state,
            SessionCommandStateResult::State(state)
                if state.session.resource != update.session
        ) {
            return false;
        }
        let Some(pane) = self.panes.get_mut(&resource_key(&update.session)) else {
            return false;
        };
        if pane.attachment.session != update.session {
            return false;
        }
        match update.history {
            RetainedHistoryUpdate::Replace {
                page,
                retention_gap,
            } => {
                if page.session != update.session {
                    return false;
                }
                pane.retained_events = page.events;
                pane.retained_cursor = Some(page.terminal_cursor);
                pane.retained_has_more = page.has_more;
                pane.retained_resynchronized = retention_gap;
                pane.retained_refusal = None;
            }
            RetainedHistoryUpdate::Append(page) => {
                if page.session != update.session || pane.retained_cursor != Some(page.from_cursor)
                {
                    return false;
                }
                pane.retained_events.extend(page.events);
                pane.retained_cursor = Some(page.terminal_cursor);
                pane.retained_has_more = page.has_more;
                pane.retained_resynchronized = false;
                pane.retained_refusal = None;
            }
            RetainedHistoryUpdate::Refused {
                outcome,
                explanation,
            } => pane.retained_refusal = Some((outcome, explanation)),
        }
        match update.command_state {
            SessionCommandStateResult::State(state) if state.session.resource == update.session => {
                if pane.mutation_fence.is_some_and(|fence| {
                    state.session.freshness.state == FreshnessState::Fresh
                        && state.session.freshness.revision > fence
                }) {
                    pane.mutation_fence = None;
                }
                pane.command_state = Some(state);
            }
            SessionCommandStateResult::Refused {
                outcome,
                explanation,
            } => pane.retained_refusal = Some((outcome, explanation)),
            SessionCommandStateResult::State(_) => return false,
        }
        true
    }

    pub fn apply_follow_up_result(&mut self, result: &PlatformFollowUpResult) -> bool {
        let follow_up = match result {
            PlatformFollowUpResult::Receipt { follow_up, .. }
            | PlatformFollowUpResult::Refused { follow_up, .. }
            | PlatformFollowUpResult::ReconciliationPending { follow_up, .. } => follow_up,
        };
        let Some(pane) = self
            .panes
            .get_mut(&resource_key(&follow_up.request.session))
        else {
            return false;
        };
        if pane.attachment.session != follow_up.request.session
            || pane.pending_follow_up.as_ref() != Some(follow_up)
        {
            return false;
        }
        match result {
            PlatformFollowUpResult::Receipt { receipt, .. } => {
                pane.pending_follow_up = None;
                pane.mutation_fence = matches!(
                    receipt.outcome,
                    ReceiptOutcome::Accepted | ReceiptOutcome::Completed
                )
                .then_some(follow_up.request.expected_session_revision);
                pane.retained_refusal = None;
            }
            PlatformFollowUpResult::Refused {
                outcome,
                explanation,
                ..
            } if *outcome != ReceiptOutcome::Unknown => {
                pane.pending_follow_up = None;
                pane.retained_refusal = Some((*outcome, explanation.clone()));
            }
            PlatformFollowUpResult::Refused {
                outcome,
                explanation,
                ..
            } => pane.retained_refusal = Some((*outcome, explanation.clone())),
            PlatformFollowUpResult::ReconciliationPending { .. } => {}
        }
        true
    }

    pub fn attachments(&self) -> impl ExactSizeIterator<Item = &Attachment> {
        self.panes.values().map(|pane| &pane.attachment)
    }

    pub fn set_lease(&mut self, lease: ControlLease) {
        if let Some(pane) = self.panes.get_mut(&resource_key(&lease.session)) {
            pane.lease = Some(lease);
            pane.control_lost = false;
            pane.error_category = None;
        }
    }

    pub fn release_lease(&mut self, session: &ResourceCoordinate) {
        if let Some(pane) = self.panes.get_mut(&resource_key(session)) {
            pane.lease = None;
            pane.control_lost = false;
        }
    }

    pub fn apply_attachment_refresh(&mut self, refresh: AttachmentRefresh) {
        match refresh {
            AttachmentRefresh::Updated { attachment, events } => {
                let key = resource_key(&attachment.session);
                if let Some(pane) = self.panes.get_mut(&key) {
                    pane.attachment = attachment;
                    pane.stream = PaneStreamState::Live;
                    pane.error_category = None;
                    if self.selected.as_deref() != Some(key.as_str()) {
                        pane.unread = pane.unread.saturating_add(events);
                    }
                }
            }
            AttachmentRefresh::Resynchronized(attachment) => {
                if let Some(pane) = self.panes.get_mut(&resource_key(&attachment.session)) {
                    pane.attachment = attachment;
                    pane.stream = PaneStreamState::Resynchronized;
                    pane.error_category = None;
                }
            }
            AttachmentRefresh::Failed {
                attachment,
                category,
            } => {
                if let Some(pane) = self.panes.get_mut(&resource_key(&attachment.session)) {
                    pane.stream = PaneStreamState::Error;
                    pane.error_category = Some(category);
                }
            }
        }
    }

    pub fn mark_online(&mut self) {
        self.online = true;
        for pane in self.panes.values_mut() {
            if pane.stream == PaneStreamState::Offline {
                pane.stream = PaneStreamState::Live;
                pane.error_category = None;
            }
        }
    }

    pub fn mark_offline(&mut self) {
        self.online = false;
        for pane in self.panes.values_mut() {
            pane.stream = PaneStreamState::Offline;
            pane.error_category = Some("offline".to_string());
            if pane.lease.take().is_some() {
                pane.control_lost = true;
            }
        }
    }

    pub fn is_online(&self) -> bool {
        self.online
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct PlatformConnection {
    endpoint: String,
    token: String,
}

impl fmt::Debug for PlatformConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlatformConnection")
            .field("endpoint", &self.endpoint)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl PlatformConnection {
    pub fn new(dashboard_url: &str, token: &str) -> Result<Self> {
        let endpoint = platform_endpoint(dashboard_url)?;
        Self::new_at_endpoint(&endpoint, token)
    }

    /// Connect to a gateway whose canonical Platform route is namespaced.
    ///
    /// Manage uses `/api/manage/automonique/platform` because Bext itself owns
    /// `/api/platform` for its deployment control API.
    pub fn new_at_endpoint(endpoint_url: &str, token: &str) -> Result<Self> {
        let endpoint = explicit_platform_endpoint(endpoint_url)?;
        BearerToken::new(token.to_owned()).map_err(platform_error)?;
        Ok(Self {
            endpoint,
            token: token.to_owned(),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn client(&self) -> Result<PlatformClient<HttpsTransport>> {
        let token = BearerToken::new(self.token.clone()).map_err(platform_error)?;
        let transport =
            HttpsTransport::new(self.endpoint.clone(), token).map_err(platform_error)?;
        Ok(PlatformClient::new(transport))
    }

    fn review_client(&self) -> Result<PlatformV2Client<HttpsTransport>> {
        let token = BearerToken::new(self.token.clone()).map_err(platform_error)?;
        let transport =
            HttpsTransport::new(self.endpoint.clone(), token).map_err(platform_error)?;
        Ok(PlatformV2Client::new_https(transport))
    }

    /// Load one authenticated Platform v2 review projection for display.
    ///
    /// This lane is intentionally read-only. A review response, provider
    /// session, or attention event never grants ShellDeck local mutation
    /// authority.
    pub fn review(&self, target: &PlatformReviewTarget) -> Result<PlatformReviewLoad> {
        let mut client = self.review_client()?;
        load_review(&mut client, target)
    }

    pub fn snapshot(&self) -> Result<PlatformSnapshot> {
        let mut client = self.client()?;
        let capabilities = client.capabilities().map_err(platform_error)?;
        let resources = client.snapshot(Vec::new()).map_err(platform_error)?;
        let sessions = client
            .list_sessions(ResourceAuthority::Automonique, None)
            .map_err(platform_error)?;
        let mut view = PlatformView::default();
        view.apply_snapshot(resources.clone());
        view.apply_session_list(&sessions);
        Ok(PlatformSnapshot {
            capabilities,
            resources: resources.resources,
            cursor: resources.cursor,
            sessions: sessions.sessions,
            sessions_cursor: sessions.cursor,
            view,
        })
    }

    pub fn refresh(
        &self,
        previous: &PlatformSnapshot,
        attachments: &[Attachment],
    ) -> Result<PlatformRefresh> {
        let mut client = self.client()?;
        let mut snapshot = previous.clone();
        let mut events = 0_usize;
        let mut resynchronized = false;

        match client
            .subscribe_recoverable(Some(snapshot.cursor.clone()))
            .map_err(platform_error)?
        {
            SubscriptionResult::Page(page) => {
                match snapshot.view.apply_subscription(page) {
                    SubscriptionApply::Applied {
                        events: applied_events,
                    } => events = events.saturating_add(applied_events),
                    SubscriptionApply::ResyncRequired => {
                        apply_full_snapshot(&mut client, &mut snapshot)?;
                        resynchronized = true;
                    }
                }
                snapshot.cursor = snapshot
                    .view
                    .cursor(&snapshot.cursor)
                    .cloned()
                    .unwrap_or_else(|| snapshot.cursor.clone());
            }
            SubscriptionResult::ResyncRequired { .. } => {
                apply_full_snapshot(&mut client, &mut snapshot)?;
                resynchronized = true;
            }
        }

        let directory_cursor = snapshot.sessions_cursor.clone();
        match client
            .subscribe_recoverable(Some(directory_cursor.clone()))
            .map_err(platform_error)?
        {
            SubscriptionResult::Page(page) => {
                let changed = !page.events.is_empty();
                match snapshot.view.apply_subscription(page) {
                    SubscriptionApply::ResyncRequired => {
                        let sessions = client
                            .list_sessions(ResourceAuthority::Automonique, None)
                            .map_err(platform_error)?;
                        snapshot.view.apply_session_list(&sessions);
                        snapshot.sessions = sessions.sessions;
                        snapshot.sessions_cursor = sessions.cursor;
                        resynchronized = true;
                    }
                    SubscriptionApply::Applied {
                        events: directory_events,
                    } => {
                        events = events.saturating_add(directory_events);
                        snapshot.sessions_cursor = snapshot
                            .view
                            .cursor(&directory_cursor)
                            .cloned()
                            .unwrap_or(directory_cursor);
                        if changed {
                            let sessions = match client
                                .list_sessions_recoverable(
                                    ResourceAuthority::Automonique,
                                    Some(snapshot.sessions_cursor.clone()),
                                )
                                .map_err(platform_error)?
                            {
                                SessionListResult::Sessions(sessions) => sessions,
                                SessionListResult::ResyncRequired { .. } => {
                                    resynchronized = true;
                                    client
                                        .list_sessions(ResourceAuthority::Automonique, None)
                                        .map_err(platform_error)?
                                }
                            };
                            snapshot.view.apply_session_list(&sessions);
                            snapshot.sessions = sessions.sessions;
                            snapshot.sessions_cursor = sessions.cursor;
                        }
                    }
                }
            }
            SubscriptionResult::ResyncRequired { .. } => {
                let sessions = client
                    .list_sessions(ResourceAuthority::Automonique, None)
                    .map_err(platform_error)?;
                snapshot.view.apply_session_list(&sessions);
                snapshot.sessions = sessions.sessions;
                snapshot.sessions_cursor = sessions.cursor;
                resynchronized = true;
            }
        }

        let mut attachment_refreshes = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            snapshot.view.track_attachment(attachment);
            let cursor = snapshot
                .view
                .attachment_cursor(attachment)
                .cloned()
                .unwrap_or_else(|| attachment.cursor.clone());
            let result = client.subscribe_recoverable(Some(cursor));
            match result {
                Ok(SubscriptionResult::Page(page)) => {
                    let pane_events = page
                        .events
                        .iter()
                        .filter(|event| event.resource.resource == attachment.session)
                        .count();
                    match snapshot
                        .view
                        .apply_attachment_subscription(attachment, page)
                    {
                        SubscriptionApply::Applied { .. } => {
                            let mut updated = attachment.clone();
                            if let Some(cursor) = snapshot.view.attachment_cursor(attachment) {
                                updated.cursor = cursor.clone();
                            }
                            events = events.saturating_add(pane_events);
                            attachment_refreshes.push(AttachmentRefresh::Updated {
                                attachment: updated,
                                events: pane_events,
                            });
                        }
                        SubscriptionApply::ResyncRequired => attachment_refreshes.push(
                            reattach_after_resync(&mut client, &mut snapshot.view, attachment),
                        ),
                    }
                }
                Ok(SubscriptionResult::ResyncRequired { .. }) => attachment_refreshes.push(
                    reattach_after_resync(&mut client, &mut snapshot.view, attachment),
                ),
                Err(error) => attachment_refreshes.push(AttachmentRefresh::Failed {
                    attachment: attachment.clone(),
                    category: error.category().to_string(),
                }),
            }
        }

        let pending_receipts = snapshot
            .view
            .receipts()
            .filter(|receipt| receipt.outcome == ReceiptOutcome::Accepted)
            .map(|receipt| receipt.id.clone())
            .collect::<Vec<_>>();
        for receipt in pending_receipts {
            if let Ok(updated) = client.get_receipt(GetReceiptRequest::by_id(receipt)) {
                snapshot.view.apply_receipt(updated);
            }
        }
        snapshot.resources = snapshot.view.resources().cloned().collect();

        Ok(PlatformRefresh {
            snapshot,
            attachments: attachment_refreshes,
            events,
            resynchronized,
        })
    }

    /// Refresh retained transcript and command state with cursors independent
    /// from resource, directory, and attachment subscriptions.
    pub fn refresh_retained_sessions(
        &self,
        reads: &[RetainedSessionRead],
    ) -> Result<Vec<RetainedSessionUpdate>> {
        let mut client = self.client()?;
        reads
            .iter()
            .map(|read| {
                let (history, retention_gap) = match read.after {
                    Some(after) => match client
                        .session_history_page(
                            read.session.clone(),
                            after,
                            RETAINED_HISTORY_PAGE_LIMIT,
                        )
                        .map_err(platform_error)?
                    {
                        SessionHistoryResult::Page(page) => {
                            (RetainedHistoryUpdate::Append(page), false)
                        }
                        SessionHistoryResult::ReplaceWithSnapshot(_) => {
                            let replacement = client
                                .session_history_snapshot(
                                    read.session.clone(),
                                    RETAINED_HISTORY_PAGE_LIMIT,
                                )
                                .map_err(platform_error)?;
                            match replacement {
                                SessionHistoryResult::Page(page) => (
                                    RetainedHistoryUpdate::Replace {
                                        page,
                                        retention_gap: true,
                                    },
                                    true,
                                ),
                                SessionHistoryResult::Refused {
                                    outcome,
                                    explanation,
                                } => (
                                    RetainedHistoryUpdate::Refused {
                                        outcome,
                                        explanation,
                                    },
                                    true,
                                ),
                                SessionHistoryResult::ReplaceWithSnapshot(_) => {
                                    return Err(ShellDeckError::Connection(
                                        "retained history replacement did not converge".to_string(),
                                    ));
                                }
                            }
                        }
                        SessionHistoryResult::Refused {
                            outcome,
                            explanation,
                        } => (
                            RetainedHistoryUpdate::Refused {
                                outcome,
                                explanation,
                            },
                            false,
                        ),
                    },
                    None => match client
                        .session_history_snapshot(read.session.clone(), RETAINED_HISTORY_PAGE_LIMIT)
                        .map_err(platform_error)?
                    {
                        SessionHistoryResult::Page(page) => (
                            RetainedHistoryUpdate::Replace {
                                page,
                                retention_gap: false,
                            },
                            false,
                        ),
                        SessionHistoryResult::Refused {
                            outcome,
                            explanation,
                        } => (
                            RetainedHistoryUpdate::Refused {
                                outcome,
                                explanation,
                            },
                            false,
                        ),
                        SessionHistoryResult::ReplaceWithSnapshot(_) => {
                            return Err(ShellDeckError::Connection(
                                "initial retained history requested a replacement".to_string(),
                            ));
                        }
                    },
                };
                let _ = retention_gap;
                let command_state = client
                    .session_command_state_outcome(read.session.clone())
                    .map_err(platform_error)?;
                Ok(RetainedSessionUpdate {
                    session: read.session.clone(),
                    history,
                    command_state,
                })
            })
            .collect()
    }

    /// Send one already-prepared follow-up exactly once. Any uncertain result
    /// becomes a receipt lookup obligation; this method never replays text.
    pub fn follow_up_reconciled(&self, follow_up: PlatformFollowUp) -> PlatformFollowUpResult {
        let request = follow_up.request.clone();
        let mut client = match self.client() {
            Ok(client) => client,
            Err(error) => {
                return PlatformFollowUpResult::ReconciliationPending {
                    follow_up,
                    category: error.to_string(),
                };
            }
        };
        match client.session_follow_up_outcome(request.clone()) {
            Ok(ActionResult::Receipt(receipt)) => {
                PlatformFollowUpResult::Receipt { follow_up, receipt }
            }
            Ok(ActionResult::Refused {
                outcome,
                explanation,
            }) if outcome != ReceiptOutcome::Unknown => PlatformFollowUpResult::Refused {
                follow_up,
                outcome,
                explanation,
            },
            ambiguous => match client.reconcile_receipt_by_idempotency_key(
                request.client,
                request.idempotency_key,
                PlatformAction::FollowUp,
                request.session,
            ) {
                Ok(receipt) => PlatformFollowUpResult::Receipt { follow_up, receipt },
                Err(error) => match ambiguous {
                    Ok(ActionResult::Refused {
                        outcome,
                        explanation,
                    }) => PlatformFollowUpResult::Refused {
                        follow_up,
                        outcome,
                        explanation,
                    },
                    _ => PlatformFollowUpResult::ReconciliationPending {
                        follow_up,
                        category: error.category().to_string(),
                    },
                },
            },
        }
    }

    /// Reconcile an unresolved follow-up by its original key only.
    pub fn reconcile_follow_up(&self, follow_up: PlatformFollowUp) -> PlatformFollowUpResult {
        let request = follow_up.request.clone();
        match self.client().and_then(|mut client| {
            client
                .reconcile_receipt_by_idempotency_key(
                    request.client,
                    request.idempotency_key,
                    PlatformAction::FollowUp,
                    request.session,
                )
                .map_err(platform_error)
        }) {
            Ok(receipt) => PlatformFollowUpResult::Receipt { follow_up, receipt },
            Err(error) => PlatformFollowUpResult::ReconciliationPending {
                follow_up,
                category: error.to_string(),
            },
        }
    }

    pub fn attach(&self, session: ResourceCoordinate, client: ClientId) -> Result<Attachment> {
        self.client()?
            .attach(session, client)
            .map_err(platform_error)
    }

    pub fn execute(&self, preview: PlatformActionPreview) -> Result<ActionResult> {
        let PlatformActionPreview {
            action,
            target,
            expected_revision,
            parameter,
            idempotency_key,
        } = preview;
        let request = ExecuteRequest::new(
            action,
            target,
            idempotency_key,
            expected_revision,
            parameter,
        )
        .map_err(|_| ShellDeckError::Connection("platform action is invalid".to_string()))?;
        self.client()?
            .execute_outcome(request)
            .map_err(platform_error)
    }

    /// Execute one prepared mutation and reconcile an ambiguous outcome with
    /// the exact same idempotency key before returning uncertainty to the UI.
    pub fn execute_reconciled(&self, preview: PlatformActionPreview) -> Result<ActionResult> {
        let idempotency_key = preview.idempotency_key.clone();
        match self.execute(preview) {
            Ok(
                result @ ActionResult::Refused {
                    outcome: ReceiptOutcome::Unknown,
                    ..
                },
            ) => match self.get_receipt_by_idempotency_key(idempotency_key) {
                Ok(receipt) => Ok(ActionResult::Receipt(receipt)),
                Err(_) => Ok(result),
            },
            Ok(result) => Ok(result),
            Err(execute_error) => match self.get_receipt_by_idempotency_key(idempotency_key) {
                Ok(receipt) => Ok(ActionResult::Receipt(receipt)),
                Err(_) => Err(execute_error),
            },
        }
    }

    pub fn get_receipt(&self, receipt: ReceiptId) -> Result<ActionReceipt> {
        self.client()?
            .get_receipt(GetReceiptRequest::by_id(receipt))
            .map_err(platform_error)
    }

    pub fn get_receipt_by_idempotency_key(
        &self,
        idempotency_key: IdempotencyKey,
    ) -> Result<ActionReceipt> {
        self.client()?
            .get_receipt(GetReceiptRequest::by_idempotency_key(idempotency_key))
            .map_err(platform_error)
    }

    pub fn detach(&self, session: ResourceCoordinate, client: ClientId) -> Result<()> {
        self.client()?
            .detach(session, client)
            .map_err(platform_error)
    }

    pub fn claim_control(
        &self,
        session: ResourceCoordinate,
        client: ClientId,
    ) -> Result<ControlClaimResult> {
        self.client()?
            .claim_control_outcome(session, client, unique_key("claim"))
            .map_err(platform_error)
    }

    pub fn release_control(
        &self,
        session: ResourceCoordinate,
        client: ClientId,
        lease: ControlLeaseId,
    ) -> Result<()> {
        self.client()?
            .release_control(ReleaseControlRequest {
                session,
                client,
                lease,
                idempotency_key: unique_key("release"),
            })
            .map_err(platform_error)
    }
}

fn apply_full_snapshot(
    client: &mut PlatformClient<HttpsTransport>,
    snapshot: &mut PlatformSnapshot,
) -> Result<()> {
    let resources = client.snapshot(Vec::new()).map_err(platform_error)?;
    snapshot.view.apply_snapshot(resources.clone());
    snapshot.resources = resources.resources;
    snapshot.cursor = resources.cursor;
    Ok(())
}

fn reattach_after_resync(
    client: &mut PlatformClient<HttpsTransport>,
    view: &mut PlatformView,
    attachment: &Attachment,
) -> AttachmentRefresh {
    match client.attach(attachment.session.clone(), attachment.client.clone()) {
        Ok(replacement) => {
            view.track_attachment(&replacement);
            AttachmentRefresh::Resynchronized(replacement)
        }
        Err(error) => AttachmentRefresh::Failed {
            attachment: attachment.clone(),
            category: error.category().to_string(),
        },
    }
}

pub fn platform_endpoint(dashboard_url: &str) -> Result<String> {
    let url = Url::parse(dashboard_url.trim())
        .map_err(|_| ShellDeckError::Connection("platform dashboard URL is invalid".to_string()))?;
    if url.scheme() != "https"
        && !(cfg!(test)
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
    {
        return Err(ShellDeckError::Connection(
            "platform dashboard must use HTTPS".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        return Err(ShellDeckError::Connection(
            "platform dashboard URL contains unsupported components".to_string(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    Ok(format!("{origin}/api/platform"))
}

fn explicit_platform_endpoint(endpoint_url: &str) -> Result<String> {
    let url = Url::parse(endpoint_url.trim())
        .map_err(|_| ShellDeckError::Connection("platform endpoint URL is invalid".to_string()))?;
    if url.scheme() != "https"
        && !(cfg!(test)
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
    {
        return Err(ShellDeckError::Connection(
            "platform endpoint must use HTTPS".to_string(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ShellDeckError::Connection(
            "platform endpoint contains unsupported components".to_string(),
        ));
    }
    Ok(url.to_string())
}

pub fn stable_client_id(seed: &str) -> Result<ClientId> {
    let normalized = seed
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(96)
        .collect::<String>();
    ClientId::new(format!(
        "shelldeck-{}",
        if normalized.is_empty() {
            "desktop"
        } else {
            &normalized
        }
    ))
    .map_err(|_| ShellDeckError::Connection("platform client identity is invalid".to_string()))
}

fn resource_key(resource: &ResourceCoordinate) -> String {
    format!(
        "{}:{}:{}",
        resource.authority.as_str(),
        resource.kind.as_str(),
        resource.id.as_str()
    )
}

fn unique_key(operation: &str) -> IdempotencyKey {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    IdempotencyKey::new(format!("shelldeck-{operation}-{nanos}"))
        .expect("bounded generated platform idempotency key")
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn platform_error(error: automonique_platform_client::ClientError) -> ShellDeckError {
    ShellDeckError::Connection(format!("platform request refused: {}", error.category()))
}

fn platform_v2_error(error: PlatformV2ClientError) -> ShellDeckError {
    ShellDeckError::Connection(format!("platform v2 request refused: {}", error.category()))
}

fn load_review<T>(
    client: &mut PlatformV2Client<T>,
    target: &PlatformReviewTarget,
) -> Result<PlatformReviewLoad> {
    let offer = PlatformVersionOffer::new(vec![2]).map_err(|_| {
        ShellDeckError::Connection("platform v2 negotiation offer is invalid".to_owned())
    })?;
    match client.negotiate(offer).map_err(platform_v2_error)? {
        NegotiationResult::V2(_) => {}
        NegotiationResult::Downgraded(_) => {
            return Ok(PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
                category: "platform_v2_unavailable".to_owned(),
                explanation: "the platform endpoint did not negotiate review v2".to_owned(),
            }));
        }
        NegotiationResult::Refused(refusal) => {
            return Ok(PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
                category: refusal.category().as_str().to_owned(),
                explanation: refusal.explanation().as_str().to_owned(),
            }));
        }
    }
    match client
        .get_review(target.project.clone(), target.workspace.clone())
        .map_err(platform_v2_error)?
    {
        ReviewReadResult::Snapshot(snapshot) => Ok(PlatformReviewLoad::Available(Box::new(
            PlatformReviewSemantic::from(snapshot.as_ref()),
        ))),
        ReviewReadResult::Refused(refusal) => {
            Ok(PlatformReviewLoad::Unavailable(PlatformReviewUnavailable {
                category: refusal.category().as_str().to_owned(),
                explanation: refusal.explanation().as_str().to_owned(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use automonique_protocol::platform::{
        CursorTopic, Freshness, PlatformEvent, PlatformRequest, PlatformResponse, ResourceId,
        SessionHistoryEvidence, SessionHistoryRole, SessionHistoryText, SessionList, Snapshot,
        Subscription,
    };
    use automonique_protocol::platform_api::{PlatformRequestMessage, PlatformResponseMessage};
    use automonique_protocol::primitives::{EpochMillis, Revision};

    // SDTEST-1775
    #[test]
    fn typed_v2_loader_negotiates_then_projects_canonical_review() {
        use automonique_platform_client::platform_v2_client::testing::{
            DeterministicPlatformV2Step, DeterministicPlatformV2Transport,
        };
        use automonique_protocol::platform_v2::{
            NegotiatedPlatform, PlatformVersion, WorkContextAvailability, WorkContextIdentity,
            WorkContextTargetKind,
        };
        use automonique_protocol::platform_v2_review_api::decode_review_snapshot;
        use automonique_protocol::platform_v2_transport::{
            PlatformNegotiationResponse, PlatformV2Response,
        };

        let snapshot = decode_review_snapshot(include_bytes!(
            "../../tests/fixtures/platform-v2-review-v2.json"
        ))
        .unwrap();
        let negotiated = NegotiatedPlatform::new(
            PlatformVersion::V2,
            PlatformVersion::V2.schema(),
            WorkContextAvailability::V2Structured,
        )
        .unwrap();
        let transport = DeterministicPlatformV2Transport::new([
            DeterministicPlatformV2Step::Negotiation(PlatformNegotiationResponse::Negotiated(
                negotiated,
            )),
            DeterministicPlatformV2Step::V2(Box::new(PlatformV2Response::ReviewResult(snapshot))),
        ]);
        let mut client = PlatformV2Client::new_testing(transport);
        let target = PlatformReviewTarget {
            project: automonique_protocol::platform_v2::ProjectId::new("project-1").unwrap(),
            workspace: WorkContextIdentity::parse_local(
                WorkContextTargetKind::UserWorkspace,
                "wc_user_1",
            )
            .unwrap(),
        };

        let loaded = load_review(&mut client, &target).unwrap();

        assert!(loaded.needs_user_action());
        assert_eq!(client.transport().pending_steps(), 0);
        assert_eq!(client.transport().negotiations().len(), 1);
        assert_eq!(client.transport().requests().len(), 1);
    }

    #[test]
    fn endpoint_is_canonical_and_https_only() {
        assert_eq!(
            platform_endpoint("https://monique.example.test/dashboard").unwrap(),
            "https://monique.example.test/api/platform"
        );
        assert!(platform_endpoint("http://monique.example.test/").is_err());
        assert!(platform_endpoint("https://user@monique.example.test/").is_err());
    }

    // SDTEST-1692
    #[test]
    fn sdtest_1692_explicit_manage_endpoint_preserves_its_namespaced_route() {
        let connection = PlatformConnection::new_at_endpoint(
            "https://manage.example.test/api/manage/automonique/platform",
            "fixture-sensitive-token",
        )
        .unwrap();
        assert_eq!(
            connection.endpoint(),
            "https://manage.example.test/api/manage/automonique/platform"
        );
        assert!(PlatformConnection::new_at_endpoint(
            "https://manage.example.test/api/manage/automonique/platform?node=private",
            "fixture-sensitive-token",
        )
        .is_err());
    }

    #[test]
    fn client_identity_is_stable_bounded_and_typed() {
        let client = stable_client_id("desktop 01/@example").unwrap();
        assert_eq!(client.as_str(), "shelldeck-desktop01example");
    }

    #[test]
    fn connection_debug_never_contains_the_bearer() {
        let connection =
            PlatformConnection::new("https://monique.example.test/", "fixture-sensitive-token")
                .unwrap();
        let rendered = format!("{connection:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("fixture-sensitive-token"));
    }

    // SDTEST-1682
    #[test]
    fn cockpit_keeps_pane_cursors_unread_and_control_loss_independent() {
        let session_a = session("session-a");
        let session_b = session("session-b");
        let attachment_a = attachment(session_a.clone(), 10);
        let attachment_b = attachment(session_b.clone(), 20);
        let mut cockpit = PlatformCockpitState::default();
        cockpit.attach(attachment_a.clone());
        cockpit.attach(attachment_b.clone());
        assert!(cockpit.select(&session_a));

        let mut updated_a = attachment_a;
        updated_a.cursor.sequence = Revision::new(11).unwrap();
        cockpit.apply_attachment_refresh(AttachmentRefresh::Updated {
            attachment: updated_a,
            events: 3,
        });
        let mut updated_b = attachment_b;
        updated_b.cursor.sequence = Revision::new(21).unwrap();
        cockpit.apply_attachment_refresh(AttachmentRefresh::Updated {
            attachment: updated_b,
            events: 2,
        });
        assert_eq!(cockpit.pane(&session_a).unwrap().unread, 0);
        assert_eq!(cockpit.pane(&session_b).unwrap().unread, 2);
        assert_eq!(
            cockpit
                .pane(&session_a)
                .unwrap()
                .attachment
                .cursor
                .sequence
                .get(),
            11
        );
        assert_eq!(
            cockpit
                .pane(&session_b)
                .unwrap()
                .attachment
                .cursor
                .sequence
                .get(),
            21
        );

        cockpit.set_lease(ControlLease {
            id: ControlLeaseId::new("lease-a").unwrap(),
            session: session_a.clone(),
            client: ClientId::new("shelldeck-test").unwrap(),
            expires_at: EpochMillis::from_millis(30_000),
            revision: Revision::FIRST,
        });
        cockpit.mark_offline();
        let pane = cockpit.pane(&session_a).unwrap();
        assert_eq!(pane.stream, PaneStreamState::Offline);
        assert!(pane.lease.is_none());
        assert!(pane.control_lost);
        cockpit.mark_online();
        assert!(cockpit.pane(&session_a).unwrap().control_lost);
    }

    // SDTEST-1685
    #[test]
    fn refresh_uses_cursors_per_surface_and_reconciles_pending_receipts() {
        let session = session("session-a");
        let initial_record = record(session.clone(), 1, "open");
        let session_list = SessionList {
            sessions: vec![SessionRecord {
                session: initial_record.clone(),
                run: None,
                attachable: true,
                controllable: true,
            }],
            cursor: cursor("sessions", 10),
        };
        let mut view = PlatformView::default();
        view.apply_snapshot(Snapshot {
            resources: vec![initial_record],
            cursor: cursor("resources", 5),
        });
        view.apply_session_list(&session_list);
        let pending = receipt(ReceiptOutcome::Accepted, 2);
        view.apply_receipt(pending.clone());
        let previous = PlatformSnapshot {
            capabilities: Capabilities::platform_v1(),
            resources: view.resources().cloned().collect(),
            cursor: cursor("resources", 5),
            sessions: session_list.sessions,
            sessions_cursor: session_list.cursor,
            view,
        };
        let attachment = attachment(session.clone(), 10);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let pending_for_server = pending.clone();
        let server = thread::spawn(move || {
            for index in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let (headers, body) = read_http_request(&mut stream);
                assert!(headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-token\r\n"));
                let request = PlatformRequestMessage::from_canonical_bytes(&body).unwrap();
                let response = match (index, request.request()) {
                    (0, PlatformRequest::Subscribe(request)) => {
                        assert_eq!(request.cursor.as_ref().unwrap().topic.as_str(), "resources");
                        PlatformResponse::Subscription(Subscription {
                            events: Vec::new(),
                            cursor: cursor("resources", 5),
                        })
                    }
                    (1, PlatformRequest::Subscribe(request)) => {
                        assert_eq!(request.cursor.as_ref().unwrap().topic.as_str(), "sessions");
                        PlatformResponse::Subscription(Subscription {
                            events: Vec::new(),
                            cursor: cursor("sessions", 10),
                        })
                    }
                    (2, PlatformRequest::Subscribe(request)) => {
                        assert_eq!(request.cursor.as_ref().unwrap().sequence.get(), 10);
                        PlatformResponse::Subscription(Subscription {
                            events: vec![PlatformEvent {
                                cursor: cursor("sessions", 11),
                                resource: record(session.clone(), 2, "open"),
                            }],
                            cursor: cursor("sessions", 11),
                        })
                    }
                    (3, PlatformRequest::GetReceipt(request)) => {
                        assert_eq!(request.id.as_ref(), Some(&pending_for_server.id));
                        PlatformResponse::Receipt(receipt(ReceiptOutcome::Completed, 3))
                    }
                    _ => panic!("unexpected platform request order"),
                };
                write_http_response(&mut stream, request.request_id().clone(), response);
            }
        });

        let connection =
            PlatformConnection::new(&format!("http://{address}/dashboard"), "fixture-token")
                .unwrap();
        let refresh = connection.refresh(&previous, &[attachment]).unwrap();
        assert!(!refresh.resynchronized);
        assert_eq!(refresh.events, 1);
        assert!(matches!(
            refresh.attachments.as_slice(),
            [AttachmentRefresh::Updated { attachment, events: 1 }]
                if attachment.cursor.sequence.get() == 11
        ));
        assert_eq!(
            refresh.snapshot.view.receipt(&pending.id).unwrap().outcome,
            ReceiptOutcome::Completed
        );
        server.join().unwrap();
    }

    // SDTEST-1706
    #[test]
    fn sdtest_1706_ambiguous_execute_reconciles_with_the_retained_idempotency_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut execute_stream, _) = listener.accept().unwrap();
            let (_headers, execute_body) = read_http_request(&mut execute_stream);
            let execute_message =
                PlatformRequestMessage::from_canonical_bytes(&execute_body).unwrap();
            let PlatformRequest::Execute(execute) = execute_message.request() else {
                panic!("expected execute request");
            };
            let retained_key = execute.idempotency_key.clone();
            write!(
                execute_stream,
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();

            let (mut receipt_stream, _) = listener.accept().unwrap();
            let (_headers, receipt_body) = read_http_request(&mut receipt_stream);
            let receipt_message =
                PlatformRequestMessage::from_canonical_bytes(&receipt_body).unwrap();
            let PlatformRequest::GetReceipt(request) = receipt_message.request() else {
                panic!("expected receipt reconciliation request");
            };
            assert_eq!(request.id, None);
            assert_eq!(request.idempotency_key.as_ref(), Some(&retained_key));
            write_http_response(
                &mut receipt_stream,
                receipt_message.request_id().clone(),
                PlatformResponse::Receipt(receipt(ReceiptOutcome::Completed, 3)),
            );
        });

        let connection =
            PlatformConnection::new(&format!("http://{address}/dashboard"), "fixture-token")
                .unwrap();
        let preview = PlatformActionPreview::new(
            PlatformAction::StopRun,
            ResourceCoordinate::new(
                ResourceAuthority::Automonique,
                ResourceKind::Run,
                ResourceId::new("run-1").unwrap(),
            ),
            Some(Revision::new(2).unwrap()),
            None,
        );
        let result = connection.execute_reconciled(preview).unwrap();
        assert!(matches!(
            result,
            ActionResult::Receipt(receipt)
                if receipt.outcome == ReceiptOutcome::Completed
        ));
        server.join().unwrap();
    }

    // SDTEST-1726
    #[test]
    fn sdtest_1726_follow_up_requires_exact_control_and_advancing_session_revision() {
        let target = session("session-a");
        let mut cockpit = PlatformCockpitState::default();
        cockpit.attach(attachment(target.clone(), 1));
        cockpit.set_lease(ControlLease {
            id: ControlLeaseId::new("lease-a").unwrap(),
            session: target.clone(),
            client: ClientId::new("shelldeck-test").unwrap(),
            expires_at: EpochMillis::from_millis(unix_millis().saturating_add(30_000)),
            revision: Revision::FIRST,
        });
        assert!(cockpit.apply_retained_update(retained_update(
            target.clone(),
            2,
            RetainedHistoryUpdate::Replace {
                page: history_page(target.clone(), 0, 1, "hello"),
                retention_gap: false,
            },
        )));

        let follow_up = cockpit
            .prepare_follow_up(&target, "continue safely")
            .unwrap();
        assert_eq!(follow_up.request.session, target);
        assert_eq!(follow_up.request.client.as_str(), "shelldeck-test");
        assert_eq!(follow_up.request.expected_session_revision.get(), 2);
        assert_eq!(follow_up.request.text.as_str(), "continue safely");
        assert!(cockpit.prepare_follow_up(&target, "too soon").is_err());

        let receipt = follow_up_receipt(target.clone(), ReceiptOutcome::Accepted, 2);
        assert!(
            cockpit.apply_follow_up_result(&PlatformFollowUpResult::Receipt { follow_up, receipt })
        );
        assert!(cockpit.prepare_follow_up(&target, "still fenced").is_err());
        assert!(cockpit.apply_retained_update(retained_update(
            target.clone(),
            2,
            RetainedHistoryUpdate::Append(empty_history_page(target.clone(), 1)),
        )));
        assert!(cockpit.prepare_follow_up(&target, "same revision").is_err());
        let mut stale_update = retained_update(
            target.clone(),
            3,
            RetainedHistoryUpdate::Append(empty_history_page(target.clone(), 1)),
        );
        let SessionCommandStateResult::State(state) = &mut stale_update.command_state else {
            unreachable!();
        };
        state.session.freshness.state = FreshnessState::Stale;
        assert!(cockpit.apply_retained_update(stale_update));
        assert!(cockpit
            .prepare_follow_up(&target, "stale revision")
            .is_err());
        assert!(cockpit.apply_retained_update(retained_update(
            target.clone(),
            3,
            RetainedHistoryUpdate::Append(empty_history_page(target.clone(), 1)),
        )));
        assert!(cockpit
            .prepare_follow_up(&target, "revision advanced")
            .is_ok());
    }

    // SDTEST-1727
    #[test]
    fn sdtest_1727_retention_gap_replaces_only_the_exact_transcript() {
        let target = session("session-a");
        let other = session("session-b");
        let mut cockpit = PlatformCockpitState::default();
        cockpit.attach(attachment(target.clone(), 1));
        cockpit.attach(attachment(other.clone(), 1));
        assert!(cockpit.apply_retained_update(retained_update(
            target.clone(),
            1,
            RetainedHistoryUpdate::Replace {
                page: history_page(target.clone(), 0, 1, "old"),
                retention_gap: false,
            },
        )));
        assert!(cockpit.apply_retained_update(retained_update(
            target.clone(),
            5,
            RetainedHistoryUpdate::Replace {
                page: history_page(target.clone(), 4, 5, "replacement"),
                retention_gap: true,
            },
        )));
        let pane = cockpit.pane(&target).unwrap();
        assert_eq!(pane.retained_events.len(), 1);
        assert_eq!(pane.retained_events[0].cursor(), 5);
        assert_eq!(pane.retained_cursor, Some(5));
        assert!(pane.retained_resynchronized);
        assert!(cockpit.pane(&other).unwrap().retained_events.is_empty());

        let stale = RetainedSessionUpdate {
            session: other,
            history: RetainedHistoryUpdate::Replace {
                page: history_page(target.clone(), 5, 6, "wrong identity"),
                retention_gap: true,
            },
            command_state: SessionCommandStateResult::State(command_state(target.clone(), 6)),
        };
        assert!(!cockpit.apply_retained_update(stale));
        cockpit.retain_directory_sessions(&[]);
        assert!(cockpit.panes().len() == 0);
        assert!(!cockpit.apply_retained_update(retained_update(
            target,
            7,
            RetainedHistoryUpdate::Replace {
                page: history_page(session("session-a"), 6, 7, "late"),
                retention_gap: false,
            },
        )));
    }

    // SDTEST-1728
    #[test]
    fn sdtest_1728_ambiguous_follow_up_reconciles_without_replaying_text() {
        let target = session("session-a");
        let target_for_server = target.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut follow_up_stream, _) = listener.accept().unwrap();
            let (_, body) = read_http_request(&mut follow_up_stream);
            let message = PlatformRequestMessage::from_canonical_bytes(&body).unwrap();
            let PlatformRequest::SessionFollowUp(request) = message.request() else {
                panic!("expected dedicated session follow-up request");
            };
            assert_eq!(request.session, target_for_server);
            assert_eq!(request.expected_session_revision.get(), 7);
            assert_eq!(request.text.as_str(), "continue");
            let key = request.idempotency_key.clone();
            write!(
                follow_up_stream,
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();

            let (mut receipt_stream, _) = listener.accept().unwrap();
            let (_, body) = read_http_request(&mut receipt_stream);
            let message = PlatformRequestMessage::from_canonical_bytes(&body).unwrap();
            let PlatformRequest::GetReceipt(request) = message.request() else {
                panic!("expected receipt lookup, not a replay");
            };
            assert_eq!(request.idempotency_key.as_ref(), Some(&key));
            assert_eq!(request.client.as_ref().unwrap().as_str(), "shelldeck-test");
            write_http_response(
                &mut receipt_stream,
                message.request_id().clone(),
                PlatformResponse::Receipt(follow_up_receipt(
                    target_for_server,
                    ReceiptOutcome::Completed,
                    8,
                )),
            );
        });

        let connection =
            PlatformConnection::new(&format!("http://{address}/dashboard"), "fixture-token")
                .unwrap();
        let follow_up = PlatformFollowUp {
            request: SessionFollowUpRequest {
                client: ClientId::new("shelldeck-test").unwrap(),
                session: target,
                expected_session_revision: Revision::new(7).unwrap(),
                idempotency_key: IdempotencyKey::new("follow-up-key").unwrap(),
                text: PlatformParameter::new("continue").unwrap(),
            },
        };
        assert!(matches!(
            connection.follow_up_reconciled(follow_up),
            PlatformFollowUpResult::Receipt { receipt, .. }
                if receipt.outcome == ReceiptOutcome::Completed
        ));
        server.join().unwrap();
    }

    fn session(id: &str) -> ResourceCoordinate {
        ResourceCoordinate::new(
            ResourceAuthority::Automonique,
            ResourceKind::Session,
            ResourceId::new(id).unwrap(),
        )
    }

    fn attachment(session: ResourceCoordinate, sequence: u64) -> Attachment {
        Attachment {
            session,
            client: ClientId::new("shelldeck-test").unwrap(),
            cursor: PlatformCursor {
                authority: ResourceAuthority::Automonique,
                topic: CursorTopic::new("sessions").unwrap(),
                sequence: Revision::new(sequence).unwrap(),
            },
        }
    }

    fn cursor(topic: &str, sequence: u64) -> PlatformCursor {
        PlatformCursor {
            authority: ResourceAuthority::Automonique,
            topic: CursorTopic::new(topic).unwrap(),
            sequence: Revision::new(sequence).unwrap(),
        }
    }

    fn record(resource: ResourceCoordinate, revision: u64, summary: &str) -> ResourceRecord {
        ResourceRecord {
            resource,
            freshness: Freshness {
                state: FreshnessState::Fresh,
                observed_at: EpochMillis::from_millis(revision as i64),
                revision: Revision::new(revision).unwrap(),
            },
            summary: PlatformText::new(summary).unwrap(),
        }
    }

    fn receipt(outcome: ReceiptOutcome, revision: u64) -> ActionReceipt {
        ActionReceipt {
            id: ReceiptId::new("receipt-1").unwrap(),
            action: PlatformAction::StopRun,
            target: ResourceCoordinate::new(
                ResourceAuthority::Automonique,
                ResourceKind::Run,
                ResourceId::new("run-1").unwrap(),
            ),
            outcome,
            revision: Revision::new(revision).unwrap(),
            recorded_at: EpochMillis::from_millis(revision as i64),
            explanation: None,
        }
    }

    fn follow_up_receipt(
        target: ResourceCoordinate,
        outcome: ReceiptOutcome,
        revision: u64,
    ) -> ActionReceipt {
        ActionReceipt {
            id: ReceiptId::new("follow-up-receipt").unwrap(),
            action: PlatformAction::FollowUp,
            target,
            outcome,
            revision: Revision::new(revision).unwrap(),
            recorded_at: EpochMillis::from_millis(revision as i64),
            explanation: None,
        }
    }

    fn history_page(
        target: ResourceCoordinate,
        from: u64,
        cursor: u64,
        text: &str,
    ) -> SessionHistoryPage {
        SessionHistoryPage::new(
            target,
            128,
            128,
            from,
            cursor,
            false,
            vec![SessionHistoryEvent::Message {
                cursor,
                at: EpochMillis::from_millis(cursor as i64),
                evidence: SessionHistoryEvidence::Authoritative,
                role: SessionHistoryRole::Assistant,
                text: SessionHistoryText::new(text).unwrap(),
                truncated: false,
            }],
        )
        .unwrap()
    }

    fn empty_history_page(target: ResourceCoordinate, cursor: u64) -> SessionHistoryPage {
        SessionHistoryPage::new(target, 128, 128, cursor, cursor, false, Vec::new()).unwrap()
    }

    fn command_state(target: ResourceCoordinate, revision: u64) -> SessionCommandState {
        SessionCommandState::new(
            record(target, revision, "retained session"),
            None,
            Vec::new(),
        )
        .unwrap()
    }

    fn retained_update(
        target: ResourceCoordinate,
        revision: u64,
        history: RetainedHistoryUpdate,
    ) -> RetainedSessionUpdate {
        RetainedSessionUpdate {
            session: target.clone(),
            history,
            command_state: SessionCommandStateResult::State(command_state(target, revision)),
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end])
            .unwrap()
            .to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap();
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&chunk[..read]);
        }
        (
            headers,
            bytes[header_end..header_end + content_length].to_vec(),
        )
    }

    fn write_http_response(
        stream: &mut std::net::TcpStream,
        request_id: automonique_protocol::codec::RequestId,
        response: PlatformResponse,
    ) {
        let body = PlatformResponseMessage::new(request_id, response)
            .to_message()
            .unwrap()
            .to_canonical_bytes();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            automonique_platform_client::PLATFORM_CONTENT_TYPE,
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    }
}
