//! Reuse the retained Platform cockpit inside Agents without mounting Fleet's navigation.
use super::*;

/// The directory summary may be lifecycle status, not a conversation title.
/// Keep a recognizable, distinct identity without pretending we received a title.
pub(crate) fn remote_session_title(record: &SessionRecord) -> String {
    let summary = record.session.summary.as_str().trim();
    if !summary.is_empty() && !matches!(summary.to_ascii_lowercase().as_str(), "open" | "closed") {
        return summary.to_owned();
    }
    let id = record.session.resource.id.as_str();
    let short = if let Some(rest) = id.strip_prefix("session_") {
        let name = rest.split('_').next().unwrap_or(rest);
        let suffix = rest
            .rsplit('_')
            .next()
            .unwrap_or(rest)
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        format!("{name} · {suffix}")
    } else {
        id.chars().take(12).collect::<String>()
    };
    t!("agents.remote.session_name", name = short).to_string()
}

impl FleetView {
    pub(crate) fn agent_sessions(&self) -> &[SessionRecord] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.sessions.as_slice())
    }

    pub(crate) fn agent_session_resume_pending(&self, session: &ResourceCoordinate) -> bool {
        self.remote_connections
            .get(&resource_key(session))
            .is_some_and(|connection| connection.pending.is_some() || connection.fence.is_some())
    }

    pub(crate) fn agent_session_resume_available(&self, session: &ResourceCoordinate) -> bool {
        self.remote_resume_supported()
            && self
                .remote_connections
                .get(&resource_key(session))
                .and_then(RemoteConnection::resume_revision)
                .is_some()
    }

    pub(crate) fn agent_sessions_loading(&self) -> bool {
        self.loading
    }

    pub(crate) fn agent_sessions_error(&self) -> Option<&str> {
        self.directory_error.as_deref()
    }

    pub(crate) fn agent_sessions_updated_at(&self) -> Option<String> {
        self.directory_updated_at
            .map(|at| at.format("%H:%M:%S").to_string())
    }

    pub(crate) fn select_agent_session(
        &mut self,
        coordinate: &ResourceCoordinate,
        cx: &mut Context<Self>,
    ) -> bool {
        // Use the full authority/kind/id, including for observation-only sessions.
        if self
            .agent_sessions()
            .iter()
            .filter(|record| &record.session.resource == coordinate)
            .count()
            != 1
        {
            return false;
        }
        self.select_session(coordinate, cx);
        if self.needs_remote_connection_check(coordinate) {
            cx.emit(FleetViewEvent::CheckSession(coordinate.clone()));
        }
        cx.notify();
        true
    }

    pub(crate) fn render_agent_session(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.tasks.open {
            return div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .child(self.render_task_toolbar(cx))
                .child(self.render_tasks(cx))
                .children(self.task_confirmation())
                .into_any_element();
        }
        let mut content = div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .flex()
            .flex_col();
        if let Some(error) = &self.error {
            content = content.child(
                Alert::error()
                    .title(t!("fleet.error.title").to_string())
                    .description(error.clone()),
            );
        }
        if let Some((outcome, explanation)) = &self.refusal {
            content = content.child(
                Alert::error()
                    .title(t!("fleet.action.refused", outcome = outcome).to_string())
                    .description(explanation.clone()),
            );
        }
        let selected = self
            .agent_sessions()
            .iter()
            .find(|record| self.session_is_open_exact(&record.session.resource));
        if let Some(record) = selected {
            content = content.child(div().flex_shrink_0().child(self.render_session(record, cx)));
            if self.cockpit.pane(&record.session.resource).is_some() {
                content = content.child(self.render_selected_pane(cx));
            } else {
                content = content.child(self.render_remote_connection(record, cx));
            }
        } else {
            content = content.child(
                div()
                    .p(px(24.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("agents.remote.unavailable").to_string()),
            );
        }
        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(self.render_action_preview(true, cx))
            .child(self.render_remote_resume_confirmation(cx))
            .child(content)
            .into_any_element()
    }
}

use shelldeck_core::config::platform::{
    stable_client_id, FreshnessState, IdempotencyKey, PlatformParameter, ReceiptOutcome,
    RetainedHistoryUpdate, Revision, SessionCommandState, SessionCommandStateResult,
    SessionFollowUpRequest,
};

/// A retained conversation is readable/resumable without inventing a live
/// attachment or a control lease. The server's typed command state owns resume.
pub(super) struct RemoteConnection {
    pub(super) session: ResourceCoordinate,
    events: Vec<SessionHistoryEvent>,
    pub(super) cursor: Option<u64>,
    has_more: bool,
    pub(super) state: Option<SessionCommandState>,
    detail: Option<String>,
    pub(super) pending: Option<PlatformFollowUp>,
    fence: Option<Revision>,
}

impl RemoteConnection {
    fn new(session: ResourceCoordinate) -> Self {
        Self {
            session,
            events: Vec::new(),
            cursor: None,
            has_more: false,
            state: None,
            detail: None,
            pending: None,
            fence: None,
        }
    }

    pub(super) fn apply(&mut self, update: RetainedSessionUpdate) {
        if update.session != self.session {
            return;
        }
        self.state = None;
        self.detail = None;
        match update.history {
            RetainedHistoryUpdate::Replace { page, .. } if page.session == self.session => {
                self.events = page.events;
                self.cursor = Some(page.terminal_cursor);
                self.has_more = page.has_more;
            }
            RetainedHistoryUpdate::Append(page)
                if page.session == self.session && self.cursor == Some(page.from_cursor) =>
            {
                self.events.extend(page.events);
                self.cursor = Some(page.terminal_cursor);
                self.has_more = page.has_more;
            }
            RetainedHistoryUpdate::Refused { explanation, .. } => {
                self.detail = Some(explanation.as_str().to_owned());
            }
            _ => return,
        }
        match update.command_state {
            SessionCommandStateResult::State(state) if state.session.resource == self.session => {
                if self.fence.is_some_and(|revision| {
                    state.session.freshness.state == FreshnessState::Fresh
                        && state.session.freshness.revision > revision
                }) {
                    self.fence = None;
                }
                self.state = Some(state);
            }
            SessionCommandStateResult::Refused { explanation, .. } => {
                self.detail = Some(explanation.as_str().to_owned());
            }
            _ => {}
        }
    }

    pub(super) fn resume_revision(&self) -> Option<Revision> {
        if self.pending.is_some() || self.fence.is_some() {
            return None;
        }
        self.state
            .as_ref()
            .filter(|state| {
                state.session.resource == self.session
                    && state.session.freshness.state == FreshnessState::Fresh
                    && state.session.summary.as_str() == "open"
            })
            .map(|state| state.session.freshness.revision)
    }
}

fn remote_connection_status(
    connection: Option<&RemoteConnection>,
    checking: bool,
    supported: bool,
) -> &'static str {
    if checking {
        "agents.remote.checking"
    } else if connection
        .is_some_and(|connection| connection.pending.is_some() || connection.fence.is_some())
    {
        "agents.remote.resume_pending"
    } else if connection
        .and_then(RemoteConnection::resume_revision)
        .is_some()
    {
        if supported {
            "agents.remote.resume_ready"
        } else {
            "agents.remote.gateway_upgrade"
        }
    } else if connection.is_some() {
        "agents.remote.resume_unavailable"
    } else {
        "agents.remote.connect_intro"
    }
}

impl FleetView {
    pub(crate) fn needs_remote_connection_check(&self, session: &ResourceCoordinate) -> bool {
        self.cockpit.pane(session).is_none()
            && !self.agent_session_resume_pending(session)
            && self.checking_remote_session.as_ref() != Some(session)
            && self
                .agent_sessions()
                .iter()
                .filter(|record| record.session.resource == *session)
                .count()
                == 1
    }

    pub(crate) fn begin_remote_connection_check(&mut self, session: &ResourceCoordinate) -> bool {
        if self.cockpit.pane(session).is_some()
            || self.agent_session_resume_pending(session)
            || !self
                .agent_sessions()
                .iter()
                .any(|record| record.session.resource == *session)
        {
            return false;
        }
        if self.operation_busy {
            // Keep the latest selection while another check or operation finishes.
            // A duplicate of the check already running does not add another request.
            if self.checking_remote_session.as_ref() != Some(session) {
                self.queued_remote_check = Some(session.clone());
            }
            return false;
        }
        self.operation_busy = true;
        self.checking_remote_session = Some(session.clone());
        self.queued_remote_check = None;
        true
    }

    pub(crate) fn request_directory_refresh(&mut self) {
        self.directory_refresh_requested = true;
    }

    pub(crate) fn directory_refresh_requested(&self) -> bool {
        self.directory_refresh_requested
    }

    pub(crate) fn take_directory_refresh_request(&mut self) -> bool {
        std::mem::take(&mut self.directory_refresh_requested)
    }

    pub(crate) fn queue_selected_remote_check(&mut self) {
        self.queued_remote_check = self.selected_session.clone();
    }

    pub(crate) fn dispatch_queued_remote_check(&mut self, cx: &mut Context<Self>) {
        if self.operation_busy || self.loading {
            return;
        }
        if let Some(session) = self.queued_remote_check.take() {
            if self.selected_session.as_ref() == Some(&session)
                && self.needs_remote_connection_check(&session)
            {
                cx.emit(FleetViewEvent::CheckSession(session));
            }
        }
    }

    pub fn set_remote_capabilities(
        &mut self,
        capabilities: shelldeck_core::config::platform::Capabilities,
    ) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.capabilities = capabilities;
        }
    }

    pub(crate) fn set_remote_read_error(&mut self, message: String) {
        self.remote_connections
            .values_mut()
            .for_each(|connection| connection.state = None);
        self.pending_remote_resume = None;
        self.error = Some(message);
    }

    pub fn set_remote_connection_results(&mut self, updates: Vec<RetainedSessionUpdate>) {
        self.checking_remote_session = None;
        for update in updates {
            if self.cockpit.pane(&update.session).is_some()
                || !self
                    .agent_sessions()
                    .iter()
                    .any(|record| record.session.resource == update.session)
            {
                continue;
            }
            self.remote_connections
                .entry(resource_key(&update.session))
                .or_insert_with(|| RemoteConnection::new(update.session.clone()))
                .apply(update);
        }
        self.operation_busy = false;
        self.error = None;
        self.refusal = None;
    }

    pub(super) fn apply_remote_follow_up_result(&mut self, result: &PlatformFollowUpResult) {
        let follow_up = match result {
            PlatformFollowUpResult::Receipt { follow_up, .. }
            | PlatformFollowUpResult::Refused { follow_up, .. }
            | PlatformFollowUpResult::ReconciliationPending { follow_up, .. } => follow_up,
        };
        let Some(connection) = self
            .remote_connections
            .get_mut(&resource_key(&follow_up.request.session))
        else {
            return;
        };
        if connection.pending.as_ref() != Some(follow_up) {
            return;
        }
        match result {
            PlatformFollowUpResult::Receipt { receipt, .. } => {
                connection.pending = None;
                connection.fence = matches!(
                    receipt.outcome,
                    ReceiptOutcome::Accepted | ReceiptOutcome::Completed
                )
                .then_some(follow_up.request.expected_session_revision);
                connection.state = None; // Re-read the server before another send.
                connection.detail = Some(format!(
                    "{} · {}",
                    t!("agents.remote.resume_received"),
                    receipt.outcome.as_str()
                ));
            }
            PlatformFollowUpResult::Refused {
                outcome,
                explanation,
                ..
            } => {
                if *outcome != ReceiptOutcome::Unknown {
                    connection.pending = None;
                }
                connection.state = None;
                connection.detail = Some(explanation.as_str().to_owned());
            }
            PlatformFollowUpResult::ReconciliationPending { .. } => {}
        }
    }

    fn remote_resume_supported(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .capabilities
                .methods
                .contains(&shelldeck_core::config::platform::PlatformMethod::SessionFollowUp)
        })
    }

    fn remote_resume_preview(&mut self, session: &ResourceCoordinate) {
        if !self.remote_resume_supported()
            || self.operation_busy
            || self.loading
            || self.selected_session.as_ref() != Some(session)
            || !self
                .agent_sessions()
                .iter()
                .any(|record| record.session.resource == *session)
        {
            return;
        }
        let Some(revision) = self
            .remote_connections
            .get(&resource_key(session))
            .and_then(RemoteConnection::resume_revision)
        else {
            return;
        };
        let text = self.composer_value.trim();
        if text.is_empty() {
            return;
        }
        let (Ok(client), Ok(text), Ok(idempotency_key)) = (
            stable_client_id(&shelldeck_core::util::hostname()),
            PlatformParameter::new(text.to_owned()),
            IdempotencyKey::new(format!("resume-{}", uuid::Uuid::new_v4())),
        ) else {
            return;
        };
        self.pending_remote_resume = Some(PlatformFollowUp {
            request: SessionFollowUpRequest {
                client,
                session: session.clone(),
                expected_session_revision: revision,
                text,
                idempotency_key,
            },
        });
    }

    fn confirm_remote_resume(&mut self, preview: &PlatformFollowUp) -> Option<PlatformFollowUp> {
        if !self.remote_resume_supported()
            || self.operation_busy
            || self.loading
            || self.pending_remote_resume.as_ref() != Some(preview)
            || self.selected_session.as_ref() != Some(&preview.request.session)
            || !self
                .agent_sessions()
                .iter()
                .any(|record| record.session.resource == preview.request.session)
        {
            return None;
        }
        let connection = self
            .remote_connections
            .get_mut(&resource_key(&preview.request.session))?;
        if connection.resume_revision() != Some(preview.request.expected_session_revision) {
            self.pending_remote_resume = None;
            return None;
        }
        connection.pending = Some(preview.clone());
        self.pending_remote_resume = None;
        self.operation_busy = true;
        Some(preview.clone())
    }

    fn render_remote_resume_confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(preview) = &self.pending_remote_resume else {
            return div().into_any_element();
        };
        let preview = preview.clone();
        div()
            .p(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::warning())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(t!("agents.remote.resume_confirm").to_string())
            .child(
                div()
                    .text_size(px(12.0))
                    .whitespace_normal()
                    .child(preview.request.text.as_str().to_owned()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        Button::new(
                            "remote-resume-confirm",
                            t!("fleet.action.confirm").to_string(),
                        )
                        .disabled(self.operation_busy || self.loading)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(follow_up) = this.confirm_remote_resume(&preview) {
                                cx.emit(FleetViewEvent::FollowUp(follow_up));
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(
                            "remote-resume-cancel",
                            t!("fleet.action.cancel").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pending_remote_resume = None;
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }

    fn disconnect_remote_conversation(&mut self, session: &ResourceCoordinate) {
        if self.operation_busy || self.agent_session_resume_pending(session) {
            return;
        }
        self.remote_connections.remove(&resource_key(session));
        if self
            .pending_remote_resume
            .as_ref()
            .is_some_and(|preview| preview.request.session == *session)
        {
            self.pending_remote_resume = None;
        }
    }

    fn render_remote_connection(
        &self,
        record: &SessionRecord,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let coordinate = record.session.resource.clone();
        let connection = self.remote_connections.get(&resource_key(&coordinate));
        let ready = self.remote_resume_supported()
            && connection
                .and_then(RemoteConnection::resume_revision)
                .is_some();
        let pending = connection
            .is_some_and(|connection| connection.pending.is_some() || connection.fence.is_some());
        let checking = self.checking_remote_session.as_ref() == Some(&coordinate);
        let status = remote_connection_status(connection, checking, self.remote_resume_supported());
        let mut body = div().flex_1().min_h_0().min_w_0().flex().flex_col();
        let check_coordinate = coordinate.clone();
        let mut header = div()
            .p(px(18.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("agents.remote.connect_title").to_string()),
            )
            .child(
                div()
                    .max_w(px(700.0))
                    .text_size(px(12.0))
                    .whitespace_normal()
                    .child(t!(status).to_string()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        Button::new(
                            "remote-connect-check",
                            t!(if connection.is_some() {
                                "agents.remote.check_again"
                            } else {
                                "agents.remote.check"
                            })
                            .to_string(),
                        )
                        .icon(IconSource::from("refresh-cw"))
                        .size(ButtonSize::Sm)
                        .disabled(checking || pending)
                        .loading(checking)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.selected_session.as_ref() == Some(&check_coordinate)
                                && this
                                    .agent_sessions()
                                    .iter()
                                    .any(|record| record.session.resource == check_coordinate)
                            {
                                cx.emit(FleetViewEvent::CheckSession(check_coordinate.clone()));
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            "remote-copy-session",
                            t!("agents.remote.copy_id").to_string(),
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                coordinate.id.as_str().to_owned(),
                            ))
                        }),
                    ),
            );
        if connection.is_some() {
            let disconnect = record.session.resource.clone();
            header = header.child(
                div().flex().child(
                    Button::new(
                        "remote-disconnect",
                        t!("agents.remote.disconnect").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .tooltip(t!("agents.remote.disconnect_hint").to_string())
                    .disabled(self.operation_busy || pending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.selected_session.as_ref() == Some(&disconnect) {
                            this.disconnect_remote_conversation(&disconnect);
                            cx.notify();
                        }
                    })),
                ),
            );
        }
        if let Some(detail) = connection.and_then(|connection| connection.detail.as_ref()) {
            header = header.child(
                div()
                    .text_size(px(11.0))
                    .whitespace_normal()
                    .text_color(ShellDeckColors::text_muted())
                    .child(detail.clone()),
            );
        }
        body = body.child(header);
        if let Some(connection) = connection {
            body = body.child(
                div()
                    .id("remote-retained-history")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .p(px(18.0))
                    .children(connection.events.iter().map(Self::render_history_event))
                    .children((connection.events.is_empty() && ready).then(|| {
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("agents.remote.history_empty").to_string())
                    }))
                    .children(
                        connection
                            .has_more
                            .then(|| div().child(t!("agents.remote.loading_history").to_string())),
                    ),
            );
        } else {
            body = body.child(div().flex_1());
        }
        if ready || pending {
            let input_session = record.session.resource.clone();
            let send_session = input_session.clone();
            body = body.child(
                div()
                    .p(px(18.0))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("agents.remote.resume_notice").to_string()),
                    )
                    .child(
                        Input::new(&self.composer_state)
                            .placeholder(t!("fleet.composer.placeholder").to_string())
                            .on_change({
                                let entity = cx.entity();
                                move |value, cx| {
                                    entity.update(cx, |this, cx| {
                                        if this.selected_session.as_ref() == Some(&input_session) {
                                            this.composer_value = value.to_string();
                                            this.drafts.insert(
                                                resource_key(&input_session),
                                                value.to_string(),
                                            );
                                            this.pending_remote_resume = None;
                                            cx.notify();
                                        }
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(
                            "remote-resume-preview",
                            t!("agents.remote.resume").to_string(),
                        )
                        .disabled(
                            !ready
                                || self.operation_busy
                                || self.loading
                                || self.composer_value.trim().is_empty(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remote_resume_preview(&send_session);
                            cx.notify();
                        })),
                    ),
            );
        }
        body.into_any_element()
    }
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    use automonique_protocol::platform::{Capabilities, CursorTopic, Freshness};
    use automonique_protocol::primitives::EpochMillis;
    use shelldeck_core::config::platform::{
        PlatformCursor, PlatformView, ResourceAuthority, ResourceId, SessionHistoryPage,
    };

    fn session(id: &str) -> ResourceCoordinate {
        ResourceCoordinate::new(
            ResourceAuthority::Automonique,
            ResourceKind::Session,
            ResourceId::new(id).unwrap(),
        )
    }
    fn record(session: &ResourceCoordinate, revision: u64) -> ResourceRecord {
        ResourceRecord {
            resource: session.clone(),
            summary: PlatformText::new("open").unwrap(),
            freshness: Freshness {
                state: FreshnessState::Fresh,
                revision: Revision::new(revision).unwrap(),
                observed_at: EpochMillis::from_millis(1),
            },
        }
    }
    fn update(session: &ResourceCoordinate, revision: u64) -> RetainedSessionUpdate {
        RetainedSessionUpdate {
            session: session.clone(),
            history: RetainedHistoryUpdate::Replace {
                page: SessionHistoryPage::new(session.clone(), 128, 128, 0, 0, false, vec![])
                    .unwrap(),
                retention_gap: false,
            },
            command_state: SessionCommandStateResult::State(
                SessionCommandState::new(record(session, revision), None, vec![]).unwrap(),
            ),
        }
    }
    fn snapshot(session: &ResourceCoordinate) -> PlatformSnapshot {
        let cursor = PlatformCursor {
            authority: ResourceAuthority::Automonique,
            topic: CursorTopic::new("sessions").unwrap(),
            sequence: Revision::FIRST,
        };
        PlatformSnapshot {
            capabilities: Capabilities::platform_v1(),
            resources: vec![],
            cursor: cursor.clone(),
            sessions_cursor: cursor,
            view: PlatformView::default(),
            sessions: vec![SessionRecord {
                session: record(session, 1),
                run: None,
                attachable: false,
                controllable: false,
            }],
        }
    }

    // SDTEST-2802 — a directory entry needs a matching fresh command-state read;
    // refused/mismatched data cannot authorize resume, and revision fences persist.
    #[::core::prelude::v1::test]
    fn retained_connection_requires_server_evidence_and_new_revision() {
        for locale in ["en", "fr"] {
            for key in [
                "agents.remote.connect_title",
                "agents.remote.connect_intro",
                "agents.remote.check",
                "agents.remote.check_again",
                "agents.remote.copy_id",
                "agents.remote.resume_ready",
                "agents.remote.resume_unavailable",
                "agents.remote.resume_pending",
                "agents.remote.resume_received",
                "agents.remote.resume_confirm",
                "agents.remote.resume_notice",
                "agents.remote.resume",
                "agents.remote.loading_history",
                "agents.remote.resume_available",
                "agents.remote.resuming",
                "agents.remote.disconnect",
                "agents.remote.disconnect_hint",
            ] {
                assert_ne!(t!(key, locale = locale).to_string(), key);
            }
        }
        let target = session("saved-session");
        let mut connection = RemoteConnection::new(target.clone());
        assert_eq!(connection.resume_revision(), None);
        connection.apply(update(&session("other"), 1));
        assert_eq!(connection.resume_revision(), None);
        connection.apply(update(&target, 1));
        assert_eq!(connection.resume_revision(), Some(Revision::FIRST));
        connection.fence = Some(Revision::FIRST);
        connection.apply(update(&target, 1));
        assert_eq!(connection.resume_revision(), None);
        connection.apply(update(&target, 2));
        assert_eq!(
            connection.resume_revision(),
            Some(Revision::new(2).unwrap())
        );
        let mut refused = update(&target, 3);
        refused.command_state = SessionCommandStateResult::Refused {
            outcome: ReceiptOutcome::Rejected,
            explanation: PlatformText::new("target_not_owned").unwrap(),
        };
        connection.apply(refused);
        assert_eq!(connection.resume_revision(), None);
        let mut stale = update(&target, 4);
        if let SessionCommandStateResult::State(state) = &mut stale.command_state {
            state.session.freshness.state = FreshnessState::Stale;
        }
        connection.apply(stale);
        assert_eq!(connection.resume_revision(), None);
        let mut foreign = update(&target, 5);
        foreign.command_state = update(&session("foreign"), 5).command_state;
        connection.apply(foreign);
        assert_eq!(connection.resume_revision(), None);
    }

    // SDTEST-2803 — explicit check/review/confirm; no attachment or lease is
    // fabricated, duplicate/stale confirms fail, logout clears retained state.
    #[::core::prelude::v1::test]
    fn remote_resume_confirmation_is_exact_once_and_context_bound() {
        let mut cx = TestAppContext::single();
        let target = session("saved-session");
        let view = cx.new(FleetView::new);
        view.update(&mut cx, |view, cx| {
            view.set_snapshot(snapshot(&target));
            view.select_session(&target, cx);
            view.composer_value = "Continue the investigation".into();
            view.remote_resume_preview(&target);
            assert!(view.pending_remote_resume.is_none());
            view.set_remote_connection_results(vec![update(&target, 1)]);
            assert!(view.cockpit.pane(&target).is_none());
            view.remote_resume_preview(&target);
            let stale = view.pending_remote_resume.clone().unwrap();
            view.apply_retained_updates(vec![update(&target, 2)]);
            assert!(view.confirm_remote_resume(&stale).is_none());
            view.remote_resume_preview(&target);
            let preview = view.pending_remote_resume.clone().unwrap();
            assert_eq!(preview.request.text.as_str(), "Continue the investigation");
            assert_eq!(view.confirm_remote_resume(&preview), Some(preview.clone()));
            assert!(view.confirm_remote_resume(&preview).is_none());
            view.disconnect_remote_conversation(&target);
            assert!(view.remote_connections.contains_key(&resource_key(&target)));
            assert_eq!(view.pending_follow_ups(), vec![preview.clone()]);
            view.set_follow_up_result(
                PlatformFollowUpResult::ReconciliationPending {
                    follow_up: preview.clone(),
                    category: "timeout".into(),
                },
                cx,
            );
            view.remote_resume_preview(&target);
            assert!(view.pending_remote_resume.is_none());
            assert_eq!(view.pending_follow_ups(), vec![preview.clone()]);
            view.set_follow_up_result(
                PlatformFollowUpResult::Receipt {
                    follow_up: preview.clone(),
                    receipt: shelldeck_core::config::platform::ActionReceipt {
                        id: shelldeck_core::config::platform::ReceiptId::new("resume-receipt")
                            .unwrap(),
                        action: PlatformAction::FollowUp,
                        target: target.clone(),
                        outcome: ReceiptOutcome::Accepted,
                        revision: Revision::new(3).unwrap(),
                        recorded_at: EpochMillis::from_millis(3),
                        explanation: None,
                    },
                },
                cx,
            );
            assert!(view.composer_value.is_empty());
            assert!(!view.agent_session_resume_available(&target));
            view.apply_retained_updates(vec![update(&target, 3)]);
            assert!(view.agent_session_resume_available(&target));
            view.composer_value = "Next step".into();
            view.remote_resume_preview(&target);
            let another = view.pending_remote_resume.clone().unwrap();
            view.select_session(&session("other"), cx);
            assert!(view.confirm_remote_resume(&another).is_none());
            view.disconnect_remote_conversation(&target);
            assert!(view.retained_reads().is_empty());
            assert_eq!(
                view.drafts.get(&resource_key(&target)).unwrap(),
                "Next step"
            );
            view.set_remote_connection_results(vec![update(&target, 3)]);
            view.set_attached(
                Attachment {
                    session: target.clone(),
                    client: stable_client_id("test").unwrap(),
                    cursor: snapshot(&target).sessions_cursor,
                },
                cx,
            );
            assert!(view.remote_connections.is_empty());
            assert_eq!(view.retained_reads().len(), 1);
            view.reset();
            assert!(view.remote_connections.is_empty());
            assert!(view.pending_remote_resume.is_none());
        });
    }
    // SDTEST-2804 — live incident: a healthy runtime behind an older gateway
    // must not be described as offline, and a retry discovers the upgrade.
    #[::core::prelude::v1::test]
    fn remote_check_distinguishes_gateway_support_and_rechecks_capabilities() {
        let mut cx = TestAppContext::single();
        let target = session("saved-session");
        let view = cx.new(FleetView::new);
        view.update(&mut cx, |view, cx| {
            let mut old = snapshot(&target);
            old.capabilities.methods.retain(|method| {
                *method != shelldeck_core::config::platform::PlatformMethod::SessionFollowUp
            });
            view.set_snapshot(old);
            assert!(view.needs_remote_connection_check(&target));
            assert!(view.select_agent_session(&target, cx));
            assert!(view.cockpit.pane(&target).is_none());
            view.set_loading(true); // A background directory refresh does not block this read.
            assert!(view.begin_remote_connection_check(&target));
            assert!(!view.begin_remote_connection_check(&target));
            assert_eq!(
                remote_connection_status(None, true, false),
                "agents.remote.checking"
            );
            view.set_remote_connection_results(vec![update(&target, 1)]);
            view.set_loading(false);
            let connection = view.remote_connections.get(&resource_key(&target));
            assert_eq!(
                remote_connection_status(connection, false, false),
                "agents.remote.gateway_upgrade"
            );
            assert!(!view.agent_session_resume_available(&target));
            assert!(view.needs_remote_connection_check(&target));
            view.composer_value = "Continue".into();
            view.remote_resume_preview(&target);
            assert!(view.pending_remote_resume.is_none());
            view.set_remote_capabilities(Capabilities::platform_v1());
            assert!(view.agent_session_resume_available(&target));
            assert_eq!(
                remote_connection_status(
                    view.remote_connections.get(&resource_key(&target)),
                    false,
                    true
                ),
                "agents.remote.resume_ready"
            );
            view.remote_resume_preview(&target);
            assert!(view.pending_remote_resume.is_some());
            assert!(view.pending_follow_ups().is_empty()); // Preview does not dispatch.
        });
        for locale in ["en", "fr"] {
            for key in [
                "agents.remote.checking",
                "agents.remote.gateway_upgrade",
                "agents.remote.history_empty",
            ] {
                assert_ne!(t!(key, locale = locale).to_string(), key);
            }
        }
    }
    // SDTEST-2805 / SDUC-561 — fast navigation must not drop the last connection check.
    #[::core::prelude::v1::test]
    fn remote_checks_queue_latest_selection_and_directory_status_tracks_only_refreshes() {
        let mut app = TestAppContext::single();
        let first = session("first");
        let second = session("second");
        let last = session("last");
        let view = app.new(FleetView::new);
        let emitted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = emitted.clone();
        let _subscription = app.update(|cx| {
            cx.subscribe(&view, move |_, event, _| {
                if let FleetViewEvent::CheckSession(session) = event {
                    captured.borrow_mut().push(session.clone());
                }
            })
        });
        view.update(&mut app, |view, cx| {
            let mut directory = snapshot(&first);
            directory.sessions.extend(snapshot(&second).sessions);
            directory.sessions.extend(snapshot(&last).sessions);
            view.set_snapshot(directory);
            assert!(view.agent_sessions_updated_at().is_some());
            view.select_agent_session(&first, cx);
            assert!(view.begin_remote_connection_check(&first));
            view.select_agent_session(&second, cx);
            assert!(!view.begin_remote_connection_check(&second));
            view.select_agent_session(&last, cx);
            assert!(!view.begin_remote_connection_check(&last));
            view.request_directory_refresh();
            assert!(!view.can_refresh());
            assert!(view.directory_refresh_requested());
        });
        emitted.borrow_mut().clear();
        view.update(&mut app, |view, cx| {
            view.set_remote_connection_results(vec![update(&first, 1)]);
            assert!(view.can_refresh());
            assert!(view.take_directory_refresh_request());
            assert!(!view.take_directory_refresh_request());
            view.dispatch_queued_remote_check(cx);
        });
        assert_eq!(&*emitted.borrow(), std::slice::from_ref(&last));
        view.update(&mut app, |view, cx| {
            assert!(view.begin_remote_connection_check(&last));
            assert!(!view.begin_remote_connection_check(&last));
            view.set_operation_error("Session check failed");
            assert!(
                view.agent_sessions_error().is_none(),
                "a per-session failure is not a directory outage"
            );
            view.dispatch_queued_remote_check(cx);
            view.set_error("Directory failed");
            assert_eq!(view.agent_sessions_error(), Some("Directory failed"));
            assert!(
                view.agent_sessions_updated_at().is_some(),
                "failed refresh retains the previous success time"
            );
            view.set_snapshot(snapshot(&last));
            view.set_remote_read_error("Transcript unavailable".into());
            assert!(view.agent_sessions_error().is_none());
            assert!(view.agent_sessions_updated_at().is_some());
            assert!(!view.agent_session_resume_available(&last));
            view.queue_selected_remote_check();
            view.request_directory_refresh();
            view.reset();
            view.dispatch_queued_remote_check(cx);
            assert!(view.agent_sessions_updated_at().is_none());
            assert!(!view.directory_refresh_requested());
        });
        assert_eq!(
            emitted.borrow().len(),
            1,
            "duplicates and reset cannot enqueue a new check"
        );
        for locale in ["en", "fr"] {
            for key in [
                "agents.remote.refreshing_directory",
                "agents.remote.directory_updated",
                "agents.remote.directory_unchecked",
            ] {
                assert_ne!(t!(key, locale = locale).to_string(), key);
            }
        }
    }
}
