//! A navigator over the shared Platform directory, never a second session store.
use super::*;
use crate::fleet_view::FleetView;
use shelldeck_core::config::platform::{FreshnessState, SessionRecord};

#[derive(Default)]
pub(super) struct RemoteAgentSessionsUi {
    source: Option<Entity<FleetView>>,
    _subscription: Option<Subscription>,
    pub(super) available: bool,
    pub(super) active: bool,
    pub(super) surface_visible: bool,
}

fn matching_sessions<'a>(sessions: &'a [SessionRecord], query: &str) -> Vec<&'a SessionRecord> {
    let query = query.trim().to_lowercase();
    sessions
        .iter()
        .filter(|record| {
            query.is_empty()
                || record
                    .session
                    .summary
                    .as_str()
                    .to_lowercase()
                    .contains(&query)
                || record
                    .session
                    .resource
                    .id
                    .as_str()
                    .to_lowercase()
                    .contains(&query)
        })
        .collect()
}

impl AgentConsoleView {
    pub(crate) fn set_remote_session_source(
        &mut self,
        source: Entity<FleetView>,
        cx: &mut Context<Self>,
    ) {
        self.remote_sessions._subscription = Some(cx.observe(&source, |_, _, cx| cx.notify()));
        self.remote_sessions.source = Some(source);
    }

    pub(crate) fn set_remote_sessions_available(
        &mut self,
        available: bool,
        cx: &mut Context<Self>,
    ) {
        self.remote_sessions.available = available;
        if !available && self.remote_sessions.active {
            // Access can disappear before Workspace updates host visibility.
            // Do not briefly mark the hidden local conversation as read.
            self.remote_sessions.active = false;
            self.set_surface_visible(false, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_remote_session_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.close_navigator_overlay(cx);
        self.remote_sessions.active = active && self.remote_sessions.available;
        self.sessions.set_surface_visible(
            self.remote_sessions.surface_visible && !self.remote_sessions.active,
        );
        cx.notify();
    }

    pub(super) fn render_remote_session_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap(px(2.0));
        if !self.remote_sessions.available {
            return list.into_any_element();
        }
        let Some(source) = &self.remote_sessions.source else {
            return list.into_any_element();
        };
        let source = source.read(cx);
        let sessions = matching_sessions(
            source.agent_sessions(),
            self.navigator_search.read(cx).content(),
        );
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .pt(px(6.0))
                .pb(px(3.0))
                .child(lucide_icon("cloud", 12.0, ShellDeckColors::text_muted()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("agents.remote.title").to_string()),
                )
                .child(Badge::new(sessions.len().to_string()).variant(BadgeVariant::Secondary))
                .child(
                    Button::new("agent-remote-refresh", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Icon)
                        .icon(IconSource::from("refresh-cw"))
                        .w(px(26.0))
                        .h(px(26.0))
                        .disabled(source.agent_sessions_loading())
                        .tooltip(t!("fleet.refresh").to_string())
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(AgentConsoleEvent::RefreshRemoteSessions)
                        })),
                ),
        );
        if let Some(error) = source.agent_sessions_error() {
            list = list.child(
                div()
                    .min_w(px(0.0))
                    .truncate()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.to_owned()),
            );
        } else if sessions.is_empty() {
            let label = if source.agent_sessions_loading() {
                t!("agents.remote.loading")
            } else {
                t!("fleet.sessions.empty")
            };
            list = list.child(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(label.to_string()),
            );
        }
        for record in sessions {
            let coordinate = record.session.resource.clone();
            let selected = self.remote_sessions.active && source.session_is_open_exact(&coordinate);
            let color = match record.session.freshness.state {
                FreshnessState::Fresh => ShellDeckColors::success(),
                FreshnessState::Stale => ShellDeckColors::warning(),
                FreshnessState::Unknown => ShellDeckColors::text_muted(),
            };
            // Match the local navigator's rows; two text lines keep remote identity visible.
            let mut row = div()
                .id(SharedString::from(format!(
                    "agent-remote-{}:{}:{}",
                    coordinate.authority.as_str(),
                    coordinate.kind.as_str(),
                    coordinate.id.as_str()
                )))
                .flex()
                .items_center()
                .gap(px(7.0))
                .h(px(43.0))
                .px(px(8.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .child(div().size(px(6.0)).flex_shrink_0().rounded_full().bg(color))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.0))
                                .child(record.session.summary.as_str().to_owned()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(9.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(coordinate.id.as_str().to_owned()),
                        ),
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(AgentConsoleEvent::OpenPlatformSession(coordinate.clone()))
                }));
            if selected {
                row = row.bg(ShellDeckColors::selected_bg());
            } else {
                row = row.hover(|style| style.bg(ShellDeckColors::hover_bg()));
            }
            list = list.child(row);
        }
        list.pb(px(7.0))
            .mb(px(4.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .into_any_element()
    }

    pub(super) fn render_remote_session(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.surface_focus_requested = false;
        let mut root = div()
            .id("agent-remote-cockpit")
            .key_context("AgentConsole")
            .on_action(cx.listener(|this, _: &AgentNewChat, _, cx| this.new_session(cx)))
            .on_action(
                cx.listener(|this, _: &AgentToggleSessions, _, cx| this.toggle_navigator(cx)),
            )
            .flex()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .bg(ShellDeckColors::bg_primary())
            .text_color(ShellDeckColors::text_primary());
        if self.tools.navigator_visible {
            root = root.child(self.render_navigator(cx));
        }
        if let Some(source) = self.remote_sessions.source.clone() {
            root = root.child(source.update(cx, |view, cx| view.render_agent_session(cx)));
        }
        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        matching_sessions, AgentConsoleView, AgentStreamEvent, FleetView, FreshnessState,
        SessionRecord, Uuid,
    };
    use automonique_protocol::platform::{Capabilities, CursorTopic, Freshness};
    use automonique_protocol::primitives::{EpochMillis, Revision};
    use gpui::AppContext;
    use shelldeck_core::config::platform::{
        PlatformCursor, PlatformSnapshot, PlatformText, PlatformView, ResourceAuthority,
        ResourceCoordinate, ResourceId, ResourceKind, ResourceRecord,
    };

    fn session(
        id: &str,
        summary: &str,
        freshness: FreshnessState,
        attachable: bool,
    ) -> SessionRecord {
        SessionRecord {
            session: ResourceRecord {
                resource: ResourceCoordinate::new(
                    ResourceAuthority::Automonique,
                    ResourceKind::Session,
                    ResourceId::new(id).unwrap(),
                ),
                freshness: Freshness {
                    state: freshness,
                    observed_at: EpochMillis::EPOCH,
                    revision: Revision::FIRST,
                },
                summary: PlatformText::new(summary).unwrap(),
            },
            run: None,
            attachable,
            controllable: false,
        }
    }

    fn snapshot(sessions: Vec<SessionRecord>) -> PlatformSnapshot {
        let cursor = PlatformCursor {
            authority: ResourceAuthority::Automonique,
            topic: CursorTopic::new("sessions").unwrap(),
            sequence: Revision::FIRST,
        };
        PlatformSnapshot {
            capabilities: Capabilities::platform_v1(),
            resources: Vec::new(),
            cursor: cursor.clone(),
            sessions,
            sessions_cursor: cursor,
            view: PlatformView::default(),
        }
    }

    // SDTEST-2645 / SDUC-559
    #[test]
    fn sdtest_2645_remote_search_includes_observation_only_and_stale_sessions_without_checkout_filters(
    ) {
        let directory = vec![
            session("remote-one", "Bext deploy", FreshnessState::Fresh, true),
            session("remote-two", "Équipe support", FreshnessState::Stale, false),
            session("remote-three", "Infra", FreshnessState::Unknown, true),
        ];
        assert_eq!(matching_sessions(&directory, "").len(), 3);
        assert_eq!(
            matching_sessions(&directory, "  ÉQUIPE ")[0]
                .session
                .resource
                .id
                .as_str(),
            "remote-two"
        );
        assert_eq!(
            matching_sessions(&directory, "REMOTE-THREE")[0]
                .session
                .summary
                .as_str(),
            "Infra"
        );
        assert!(matching_sessions(&directory, "unrelated").is_empty());
    }

    // SDTEST-2646 / SDUC-559
    #[test]
    fn sdtest_2646_remote_selection_tracks_shared_directory_and_preserves_hidden_local_attention() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let source = cx.new(FleetView::new);
            let console = cx.new(AgentConsoleView::new);
            let remote = session("remote-one", "Remote agent", FreshnessState::Fresh, false);
            source.update(cx, |view, cx| {
                view.set_snapshot(snapshot(vec![remote.clone()]));
                let mut foreign = remote.session.resource.clone();
                foreign.authority = ResourceAuthority::AiOperations;
                assert!(!view.select_agent_session(&foreign, cx));
                assert!(view.select_agent_session(&remote.session.resource, cx));
                assert!(
                    view.attachments().is_empty(),
                    "selection must not claim or attach a session"
                );
            });
            console.update(cx, |view, cx| {
                view.set_remote_session_source(source.clone(), cx);
                view.set_remote_sessions_available(true, cx);
                view.set_surface_visible(true, cx);
                view.set_remote_session_active(true, cx);
                let local = view.sessions.selected_id().unwrap();
                let run = Uuid::new_v4();
                view.foreground_runs.insert(run);
                view.run_sessions.insert(run, local);
                assert!(!view.run_requests_focus(run));
                view.sessions
                    .apply_stream_event(
                        local,
                        AgentStreamEvent::Text("background reply".into()),
                        10,
                    )
                    .unwrap();
                view.set_surface_visible(true, cx); // Workspace navigation must not consume hidden replies.
                assert_eq!(view.sessions.get(local).unwrap().unread_count, 1);
                assert!(view.select_session(local, cx));
                assert!(view.run_requests_focus(run));
                assert_eq!(view.sessions.get(local).unwrap().unread_count, 0);
                assert!(!view.remote_sessions.active);
                view.set_remote_session_active(true, cx);
            });
            source.update(cx, |view, cx| {
                // A directory refresh must remove vanished sessions from Agents too.
                view.set_snapshot(snapshot(Vec::new()));
                assert!(view.agent_sessions().is_empty());
                assert!(!view.select_agent_session(&remote.session.resource, cx));
                view.reset();
                cx.notify();
            });
            console.update(cx, |view, cx| {
                let local = view.sessions.selected_id().unwrap();
                view.sessions
                    .apply_stream_event(
                        local,
                        AgentStreamEvent::Text("reply before logout".into()),
                        20,
                    )
                    .unwrap();
                view.set_remote_sessions_available(false, cx);
                view.set_remote_session_active(true, cx);
                assert!(!view.remote_sessions.active);
                assert!(!view.sessions.surface_visible());
                assert_eq!(view.sessions.get(local).unwrap().unread_count, 1);
            });
        });
    }
}
