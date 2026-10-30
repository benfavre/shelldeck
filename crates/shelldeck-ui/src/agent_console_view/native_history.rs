//! Passive native transcripts: never replay historical events into run state.
use super::*;
use shelldeck_core::agent_remote_history::{
    RemoteHistoryKind, RemoteHistoryPage, RemoteHistoryPosition, RemoteHistoryQuery,
};
use shelldeck_core::agent_remote_sessions::RemoteAgentSession;
use shelldeck_core::agent_session::RemoteHistoryReference;

#[derive(Debug, Clone)]
pub struct NativeHistoryRequest {
    pub session_id: Uuid,
    pub request_id: Uuid,
    pub connection_id: Uuid,
    pub remote: RemoteAgentSession,
    pub query: RemoteHistoryQuery,
}
#[derive(Default)]
pub(super) struct NativeHistoryUi {
    pub page: Option<RemoteHistoryPage>,
    pub fresh_view: bool,
    pending: Option<Uuid>,
    error: Option<String>,
    query: RemoteHistoryQuery,
}
fn remote(reference: &RemoteHistoryReference, title: &str) -> RemoteAgentSession {
    RemoteAgentSession {
        provider: reference.provider,
        session_id: reference.session_id.clone(),
        workdir: reference.workdir.clone(),
        title: title.into(),
        updated_at_ms: 0,
        model: reference.model.clone(),
    }
}
impl AgentConsoleView {
    /// Context edits are proposals until a separate conversation is explicitly created.
    /// Never clear the imported source's live resume authority while retaining its timeline.
    pub(super) fn validate_native_context_transition(
        &self,
        id: Uuid,
        context: &AgentExecutionContext,
    ) -> Result<(), String> {
        if self
            .sessions
            .get(id)
            .is_some_and(|session| session.remote_history.is_some() && session.context != *context)
        {
            return Err(t!("agents.native_history.context_change").to_string());
        }
        Ok(())
    }

    fn start_separate_native_draft(
        &mut self,
        source_id: Uuid,
        proposed: &AgentExecutionContext,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.selected_id() != Some(source_id)
            || self.displayed_context(cx) != *proposed
            || self.context_controls_locked()
            || self
                .validate_native_context_transition(source_id, proposed)
                .is_ok()
            || self.composer_context_loading(source_id)
        {
            return;
        }
        if let Err(error) = validate_agent_target(&proposed.target, &self.connections) {
            self.apply_session_draft_error(source_id, error, cx);
            return;
        }
        let draft = self
            .session_ui
            .get(&source_id)
            .map(|ui| ui.prompt.read(cx).content().to_string())
            .unwrap_or_default();
        match self.create_session(t!("agents.session.new").to_string(), proposed.clone(), cx) {
            Ok(id) => {
                self.session_ui[&id]
                    .prompt
                    .update(cx, |state, cx| state.replace_content(draft, cx));
                self.copy_composer_draft_context(source_id, id);
                self.schedule_library_save(cx);
                cx.notify();
            }
            Err(error) => self.apply_session_draft_error(source_id, error, cx),
        }
    }

    pub fn native_history_query(
        &self,
        connection_id: Uuid,
        remote: &RemoteAgentSession,
    ) -> RemoteHistoryQuery {
        let snapshot = self.sessions.sessions().iter().find_map(|session| {
            session
                .remote_history
                .as_ref()
                .filter(|reference| {
                    reference.connection_id == connection_id
                        && reference.provider == remote.provider
                        && reference.session_id == remote.session_id
                        && reference.workdir == remote.workdir
                })
                .and_then(|reference| reference.snapshot.clone())
        });
        RemoteHistoryQuery {
            snapshot,
            position: RemoteHistoryPosition::Latest,
        }
    }
    pub(super) fn needs_native_reconnect(&self, session: &AgentSession) -> bool {
        Self::native_context_needs_reconnect(session, &session.context)
    }
    pub(super) fn native_context_needs_reconnect(
        session: &AgentSession,
        context: &AgentExecutionContext,
    ) -> bool {
        session.remote_history.as_ref().is_some_and(|reference| {
            session.provider_session_id().is_none()
                && context.provider == reference.provider
                && context.workdir == reference.workdir
                && context.model == reference.model
                && context.access == AgentAccessMode::ReadOnly
                && matches!(&context.target, AgentTarget::Ssh{connection_id,..} if *connection_id == reference.connection_id)
        })
    }
    pub(super) fn install_native_history(&mut self, id: Uuid, page: RemoteHistoryPage) {
        let fresh_view = self.sessions.get(id).is_some_and(|session| {
            (!session.messages.is_empty() || !session.trace.is_empty())
                && session
                    .remote_history
                    .as_ref()
                    .and_then(|r| r.snapshot.as_ref())
                    .is_some_and(|snapshot| *snapshot != page.snapshot)
        });
        if !fresh_view {
            if let Some(reference) = self
                .sessions
                .get_mut(id)
                .and_then(|s| s.remote_history.as_mut())
            {
                reference.snapshot = Some(page.snapshot.clone());
            }
        }
        self.native_history.insert(
            id,
            NativeHistoryUi {
                query: RemoteHistoryQuery {
                    snapshot: Some(page.snapshot.clone()),
                    position: RemoteHistoryPosition::Latest,
                },
                page: Some(page),
                fresh_view,
                ..Default::default()
            },
        );
    }
    fn request_native_history(
        &mut self,
        id: Uuid,
        position: RemoteHistoryPosition,
        reload: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.sessions.get(id) else {
            return;
        };
        let Some(reference) = &session.remote_history else {
            return;
        };
        let state = self.native_history.entry(id).or_default();
        if state.pending.is_some() {
            return;
        }
        let query = RemoteHistoryQuery {
            snapshot: if reload {
                state.fresh_view = true;
                state.page = None;
                None
            } else if state.fresh_view {
                state.query.snapshot.clone()
            } else {
                reference.snapshot.clone()
            },
            position,
        };
        let request = NativeHistoryRequest {
            session_id: id,
            request_id: Uuid::new_v4(),
            connection_id: reference.connection_id,
            remote: remote(reference, &session.name),
            query: query.clone(),
        };
        state.pending = Some(request.request_id);
        state.query = query;
        state.error = None;
        cx.emit(AgentConsoleEvent::LoadSshHistory(request));
        cx.notify();
    }
    pub fn complete_native_history(
        &mut self,
        request: NativeHistoryRequest,
        result: Result<RemoteHistoryPage, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(reference) = self
            .sessions
            .get(request.session_id)
            .and_then(|s| s.remote_history.as_ref())
        else {
            return;
        };
        if reference.connection_id != request.connection_id
            || reference.provider != request.remote.provider
            || reference.session_id != request.remote.session_id
            || reference.workdir != request.remote.workdir
            || reference.model != request.remote.model
        {
            return;
        }
        let Some(state) = self.native_history.get_mut(&request.session_id) else {
            return;
        };
        if state.pending != Some(request.request_id) {
            return;
        }
        state.pending = None;
        match result {
            Ok(page)
                if page.provider == reference.provider
                    && page.session_id == reference.session_id
                    && page.workdir == reference.workdir =>
            {
                if !state.fresh_view {
                    self.sessions
                        .get_mut(request.session_id)
                        .unwrap()
                        .remote_history
                        .as_mut()
                        .unwrap()
                        .snapshot = Some(page.snapshot.clone());
                }
                state.query.snapshot = Some(page.snapshot.clone());
                state.page = Some(page);
                if self.sessions.selected_id() == Some(request.session_id)
                    && !self.remote_sessions.active
                    && self.ssh_sessions.preview.is_none()
                {
                    self.scroll
                        .set_offset(point(gpui::Pixels::ZERO, gpui::Pixels::ZERO));
                }
                state.error = None;
                self.schedule_library_save(cx);
            }
            Ok(_) => state.error = Some(t!("agents.ssh_sessions.session_missing").to_string()),
            Err(error) => state.error = Some(error),
        }
        cx.notify();
    }
    pub(super) fn ensure_native_history(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.sessions.selected_id() {
            if !self.native_history.contains_key(&id)
                && self
                    .sessions
                    .get(id)
                    .is_some_and(|s| s.remote_history.is_some())
            {
                self.request_native_history(id, RemoteHistoryPosition::Latest, false, cx);
            }
        }
    }
    pub(super) fn render_native_context_change(
        &self,
        session: &AgentSession,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let id = session.id;
        let proposed = self.displayed_context(cx);
        if self
            .validate_native_context_transition(id, &proposed)
            .is_err()
        {
            let expected = proposed.clone();
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .whitespace_normal()
                            .child(t!("agents.native_history.context_change").to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.0))
                            .child(
                                Button::new(
                                    "native-new-context",
                                    t!("agents.native_history.new_context").to_string(),
                                )
                                .size(ButtonSize::Sm)
                                .disabled(
                                    self.composer_context_loading(id)
                                        || self.context_controls_locked(),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.start_separate_native_draft(id, &expected, cx);
                                    },
                                )),
                            )
                            .child(
                                Button::new(
                                    "native-restore-context",
                                    t!("agents.native_history.restore_context").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.sessions.selected_id() == Some(id) {
                                            this.apply_selected_context(cx);
                                            this.composer_ui.errors.remove(&id);
                                            cx.notify();
                                        }
                                    },
                                )),
                            ),
                    )
                    .into_any_element(),
            );
        }
        None
    }

    pub(super) fn render_native_history(
        &self,
        session: &AgentSession,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = session.id;
        let state = self.native_history.get(&id);
        let busy = state.is_some_and(|s| s.pending.is_some());
        let mut view = div()
            .w_full()
            .max_w(px(800.0))
            .mx_auto()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(12.0))
            .pb(px(20.0));
        let mut bar = div().flex().flex_wrap().items_center().gap(px(8.0)).child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(t!("agents.native_history.title").to_string()),
        );
        if state.is_some_and(|state| state.fresh_view) {
            bar = bar.child(
                Button::new("history-back", t!("agents.native_history.back").to_string())
                    .size(ButtonSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.native_history.remove(&id);
                        this.request_native_history(id, RemoteHistoryPosition::Latest, false, cx);
                    })),
            );
            view = view.child(
                div()
                    .whitespace_normal()
                    .text_size(px(12.0))
                    .child(t!("agents.native_history.fresh_help").to_string()),
            );
        } else if self.needs_native_reconnect(session) {
            let reference = session.remote_history.as_ref().unwrap().clone();
            let title = session.name.clone();
            bar = bar.child(
                Button::new(
                    "history-reconnect",
                    t!("agents.native_history.reconnect").to_string(),
                )
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.retain_draft_context(cx);
                    this.ssh_sessions.preview =
                        Some((reference.connection_id, remote(&reference, &title)));
                    this.sessions.set_surface_visible(false);
                    cx.notify();
                })),
            );
        }
        bar = bar.child(
            Button::new(
                "history-fresh",
                t!("agents.native_history.fresh").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .disabled(busy)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.request_native_history(id, RemoteHistoryPosition::Latest, true, cx)
            })),
        );
        if busy {
            bar = bar.child(
                div()
                    .text_size(px(12.0))
                    .child(t!("agents.native_history.loading").to_string()),
            );
        }
        view = view.child(bar);
        if let Some(state) = state {
            if let Some(error) = &state.error {
                let position = state.query.position;
                view = view
                    .child(
                        div()
                            .whitespace_normal()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::error())
                            .child(error.clone()),
                    )
                    .child(
                        Button::new(
                            "history-retry",
                            t!("agents.native_history.retry").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_native_history(id, position, false, cx)
                        })),
                    );
            }
            if let Some(page) = &state.page {
                let start = page.start;
                let end = page.end;
                view = view.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(
                            t!(
                                "agents.native_history.range",
                                start = if page.total_entries == 0 {
                                    0
                                } else {
                                    start + 1
                                },
                                end = end,
                                total = page.total_entries
                            )
                            .to_string(),
                        ),
                );
                let controls = || {
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "history-older",
                                t!("agents.native_history.older").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(busy || start == 0)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.request_native_history(
                                        id,
                                        RemoteHistoryPosition::Before(start),
                                        false,
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                "history-newer",
                                t!("agents.native_history.newer").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(busy || end >= page.total_entries)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.request_native_history(
                                        id,
                                        RemoteHistoryPosition::After(end),
                                        false,
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                "history-latest",
                                t!("agents.native_history.latest").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(busy || end >= page.total_entries)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.request_native_history(
                                        id,
                                        RemoteHistoryPosition::Latest,
                                        false,
                                        cx,
                                    )
                                },
                            )),
                        )
                };
                view = view.child(controls());
                for warning in &page.warnings {
                    view = view.child(
                        div()
                            .text_size(px(11.0))
                            .whitespace_normal()
                            .child(warning.clone()),
                    );
                }
                for entry in &page.entries {
                    let label = match entry.kind {
                        RemoteHistoryKind::User => t!("agents.role.you").to_string(),
                        RemoteHistoryKind::Assistant => page.provider.display_name().into(),
                        _ => entry
                            .title
                            .clone()
                            .unwrap_or_else(|| t!("agents.activity.title").to_string()),
                    };
                    let label = if entry.parts > 1 {
                        format!("{label} · {}/{}", entry.part, entry.parts)
                    } else {
                        label
                    };
                    view = view.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .flex_shrink_0()
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .when(entry.kind == RemoteHistoryKind::User, |row| {
                                row.bg(ShellDeckColors::bg_surface())
                            })
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(label),
                            )
                            .child(
                                Markdown::new(entry.text.clone())
                                    .base_font_size(px(14.0).to_pixels(window.rem_size()))
                                    .compact()
                                    .w_full()
                                    .min_w_0()
                                    .whitespace_normal(),
                            ),
                    );
                }
                // Footer controls use a distinct ID scope.
                view = view.child(div().id("history-footer").child(controls()));
            }
        }
        view.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-2832 / SDUC-564 — edits cannot silently shed native resume custody;
    // only an explicit separate draft accepts a changed execution context.
    #[::core::prelude::v1::test]
    fn native_context_edits_require_explicit_separate_conversation() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let connection_id = Uuid::new_v4();
            view.set_connections(
                vec![AgentConnectionOption {
                    id: connection_id,
                    label: "Server".into(),
                    host: "fixture.invalid".into(),
                }],
                cx,
            );
            let original = AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Ssh {
                    connection_id,
                    connection_label: "Server".into(),
                },
                workdir: "/srv/repo".into(),
                access: AgentAccessMode::ReadOnly,
                model: None,
            };
            let id = view
                .sessions
                .reconnect_remote("Existing conversation", original.clone(), "native-id", 1)
                .unwrap();
            view.session_ui.insert(id, AgentSessionUi::new(cx));
            view.apply_selected_context(cx);
            view.session_ui[&id].prompt.update(cx, |state, cx| {
                state.replace_content("Keep my unsent question", cx)
            });
            assert_eq!(view.pending_from_draft(cx).unwrap().context, original);
            for change in 0..5 {
                view.apply_selected_context(cx);
                match change {
                    0 => view.change_draft_provider(AgentProvider::Claude, cx),
                    1 => view.change_draft_access(AgentAccessMode::WorkspaceWrite, cx),
                    2 => view
                        .model_state
                        .update(cx, |state, cx| state.replace_content("different-model", cx)),
                    3 => view.selected_connection = None,
                    _ => view
                        .workdir_state
                        .update(cx, |state, cx| state.replace_content("/srv/another", cx)),
                }
                let proposed = view.displayed_context(cx);
                assert_ne!(proposed, original);
                assert!(view.pending_from_draft(cx).is_err());
                view.retain_draft_context(cx);
                // Even a pre-captured/adversarial pending request cannot bypass the gate.
                view.start_pending(
                    PendingRun {
                        session_id: id,
                        context: proposed,
                        prompt: "Do not launch".into(),
                        options: Default::default(),
                        source_draft: None,
                    },
                    cx,
                );
                assert!(view.run_sessions.is_empty());
                assert_eq!(view.sessions.get(id).unwrap().context, original);
                assert_eq!(
                    view.sessions.get(id).unwrap().provider_session_id(),
                    Some("native-id")
                );
                assert!(view.sessions.get(id).unwrap().messages.is_empty());
            }
            view.apply_selected_context(cx);
            assert!(view.pending_from_draft(cx).is_ok());
            view.change_draft_access(AgentAccessMode::WorkspaceWrite, cx);
            let proposed = view.displayed_context(cx);
            view.start_separate_native_draft(id, &proposed, cx);
            let new_id = view.sessions.selected_id().unwrap();
            assert_ne!(new_id, id);
            let fresh = view.sessions.get(new_id).unwrap();
            assert_eq!(fresh.context, proposed);
            assert!(fresh.remote_history.is_none());
            assert!(fresh.provider_session_id().is_none());
            assert!(fresh.messages.is_empty());
            assert_eq!(
                view.session_ui[&new_id].prompt.read(cx).content(),
                "Keep my unsent question"
            );
            assert_eq!(
                view.session_ui[&id].prompt.read(cx).content(),
                "Keep my unsent question"
            );
            assert_eq!(view.sessions.get(id).unwrap().context, original);
            assert_eq!(
                view.sessions.get(id).unwrap().provider_session_id(),
                Some("native-id")
            );
            assert!(view.run_sessions.is_empty());
            // Ordinary unsent conversations retain the existing editable-context behavior.
            view.change_draft_provider(AgentProvider::Claude, cx);
            assert!(view.context_from_controls(cx).is_ok());
            view.retain_draft_context(cx);
            assert_eq!(
                view.sessions.get(new_id).unwrap().context.provider,
                AgentProvider::Claude
            );
        });
    }

    // SDTEST-2818 / SDUC-563 — paging is passive, source-scoped, retryable,
    // preserves local messages and rejects callbacks after account reset.
    #[::core::prelude::v1::test]
    fn native_history_paging_is_passive_and_fences_callbacks() {
        let mut app = gpui::TestAppContext::single();
        let view = app.new(AgentConsoleView::new);
        view.update(&mut app, |view, cx| {
            view.library = AgentLibraryUi::new(cx);
            let connection_id = Uuid::new_v4();
            let context = AgentExecutionContext {
                provider: AgentProvider::Codex,
                target: AgentTarget::Ssh {
                    connection_id,
                    connection_label: "Server".into(),
                },
                workdir: "/srv/repo".into(),
                access: AgentAccessMode::ReadOnly,
                model: None,
            };
            let id = view
                .sessions
                .reconnect_remote("Original", context, "native-id", 1)
                .unwrap();
            let source = view
                .sessions
                .get(id)
                .unwrap()
                .remote_history
                .clone()
                .unwrap();
            let page = RemoteHistoryPage {
                provider: source.provider,
                session_id: source.session_id.clone(),
                workdir: source.workdir.clone(),
                model: None,
                snapshot: "cutoff".into(),
                total_entries: 100,
                start: 36,
                end: 100,
                entries: vec![shelldeck_core::agent_remote_history::RemoteHistoryEntry {
                    index: 36,
                    kind: RemoteHistoryKind::Tool,
                    title: Some("approval".into()),
                    text: "Never execute this historical action".into(),
                    at_ms: None,
                    part: 1,
                    parts: 1,
                }],
                warnings: vec![],
            };
            view.install_native_history(id, page.clone());
            view.request_native_history(id, RemoteHistoryPosition::Before(36), false, cx);
            let pending = view.native_history[&id].pending.unwrap();
            let request = NativeHistoryRequest {
                session_id: id,
                request_id: pending,
                connection_id,
                remote: remote(&source, "Original"),
                query: view.native_history[&id].query.clone(),
            };
            assert_eq!(request.query.snapshot.as_deref(), Some("cutoff"));
            let mut wrong = request.clone();
            wrong.request_id = Uuid::new_v4();
            view.complete_native_history(wrong, Err("stale".into()), cx);
            assert_eq!(view.native_history[&id].pending, Some(pending));
            let mut wrong = request.clone();
            wrong.remote.session_id = "foreign-id".into();
            view.complete_native_history(wrong, Ok(page.clone()), cx);
            assert_eq!(view.native_history[&id].pending, Some(pending));
            view.complete_native_history(request.clone(), Err("offline".into()), cx);
            assert_eq!(view.native_history[&id].page.as_ref().unwrap().start, 36);
            assert!(view.native_history[&id].error.is_some());
            view.request_native_history(id, RemoteHistoryPosition::Before(36), false, cx);
            let mut request = request;
            request.request_id = view.native_history[&id].pending.unwrap();
            let mut older = page.clone();
            older.start = 0;
            older.end = 36;
            view.complete_native_history(request.clone(), Ok(older), cx);
            assert_eq!(view.native_history[&id].page.as_ref().unwrap().start, 0);
            let session = view.sessions.get(id).unwrap();
            assert!(session.messages.is_empty());
            assert!(session.trace.is_empty());
            assert!(!session.status.is_active());
            assert!(view.foreground_runs.is_empty());
            let restored: AgentSession =
                serde_json::from_str(&serde_json::to_string(session).unwrap()).unwrap();
            assert!(view.needs_native_reconnect(&restored));
            let mut changed = restored.context.clone();
            changed.model = Some("different".into());
            assert!(!AgentConsoleView::native_context_needs_reconnect(
                &restored, &changed
            ));
            view.request_native_history(id, RemoteHistoryPosition::Latest, true, cx);
            assert!(view.native_history[&id].fresh_view);
            assert!(view.native_history[&id].query.snapshot.is_none());
            let mut fresh_request = request.clone();
            fresh_request.request_id = view.native_history[&id].pending.unwrap();
            fresh_request.query = view.native_history[&id].query.clone();
            let mut refreshed = page.clone();
            refreshed.snapshot = "refreshed".into();
            view.complete_native_history(fresh_request, Ok(refreshed), cx);
            assert_eq!(
                view.sessions
                    .get(id)
                    .unwrap()
                    .remote_history
                    .as_ref()
                    .unwrap()
                    .snapshot
                    .as_deref(),
                Some("cutoff")
            );
            view.request_native_history(id, RemoteHistoryPosition::Before(36), false, cx);
            assert_eq!(
                view.native_history[&id].query.snapshot.as_deref(),
                Some("refreshed")
            );
            view.sessions
                .begin_run(id, "local retained reply", 2)
                .unwrap();
            view.sessions
                .finish(id, AgentSessionStatus::Completed, 3)
                .unwrap();
            let mut changed_page = page.clone();
            changed_page.snapshot = "changed-after-compaction".into();
            view.install_native_history(id, changed_page);
            assert!(view.native_history[&id].fresh_view);
            assert_eq!(view.sessions.get(id).unwrap().messages.len(), 1);
            assert_eq!(
                view.sessions
                    .get(id)
                    .unwrap()
                    .remote_history
                    .as_ref()
                    .unwrap()
                    .snapshot
                    .as_deref(),
                Some("cutoff")
            );
            view.reset_ssh_sessions(cx);
            view.complete_native_history(request, Ok(page), cx);
            assert!(view.native_history.is_empty());
        });
    }
}
