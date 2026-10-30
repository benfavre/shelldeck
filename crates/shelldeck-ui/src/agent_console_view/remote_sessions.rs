//! A navigator over the shared Platform directory, never a second session store.
use super::*;
use crate::fleet_view::{remote_session_title, FleetView};
use shelldeck_core::config::platform::{FreshnessState, SessionRecord};

#[derive(Default)]
pub(super) struct RemoteAgentSessionsUi {
    source: Option<Entity<FleetView>>,
    _subscription: Option<Subscription>,
    pub(super) available: bool,
    pub(super) active: bool,
    pub(super) surface_visible: bool,
    visible_limit: usize,
    page_query: String,
}

fn matching_sessions<'a>(sessions: &'a [SessionRecord], query: &str) -> Vec<&'a SessionRecord> {
    let query = query.trim().to_lowercase();
    let mut matches = sessions
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
                || record.session.resource.authority.as_str().contains(&query)
                || remote_session_title(record).to_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| {
        b.session
            .freshness
            .observed_at
            .as_millis()
            .cmp(&a.session.freshness.observed_at.as_millis())
            .then_with(|| {
                a.session
                    .resource
                    .id
                    .as_str()
                    .cmp(b.session.resource.id.as_str())
            })
            .then_with(|| {
                a.session
                    .resource
                    .authority
                    .as_str()
                    .cmp(b.session.resource.authority.as_str())
            })
    });
    matches
}

fn remote_page_limit(page_query: &str, query: &str, visible_limit: usize) -> usize {
    if page_query == query {
        visible_limit.max(20)
    } else {
        20
    }
}

fn remote_session_description(record: &SessionRecord, resumable: bool, pending: bool) -> String {
    let access = if pending {
        t!("agents.remote.resuming")
    } else if resumable {
        t!("agents.remote.resume_available")
    } else if record.attachable {
        t!("agents.remote.available")
    } else {
        t!("agents.remote.directory_only")
    };
    let freshness = match record.session.freshness.state {
        FreshnessState::Fresh => t!("agents.remote.fresh"),
        FreshnessState::Stale => t!("agents.remote.stale"),
        FreshnessState::Unknown => t!("agents.remote.unknown"),
    };
    match record
        .session
        .summary
        .as_str()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "open" => format!("{} · {access}", t!("agents.remote.open")),
        "closed" => format!("{} · {access}", t!("agents.remote.closed")),
        _ => format!("{access} · {freshness}"),
    }
}

// GPUI owns hover timing and places this above the clipped navigator;
// adabraka Tooltip currently uses an always-mounted child overlay.
struct RemoteSessionTooltip(String);
impl Render for RemoteSessionTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(8.0))
            .max_w(px(420.0))
            .rounded(px(5.0))
            .bg(ShellDeckColors::bg_surface())
            .text_color(ShellDeckColors::text_primary())
            .text_size(px(11.0))
            .child(self.0.clone())
    }
}

impl AgentConsoleView {
    pub fn set_remote_session_source(&mut self, source: Entity<FleetView>, cx: &mut Context<Self>) {
        self.remote_sessions._subscription = Some(cx.observe(&source, |_, _, cx| cx.notify()));
        self.remote_sessions.source = Some(source);
    }

    pub fn set_remote_sessions_available(&mut self, available: bool, cx: &mut Context<Self>) {
        self.remote_sessions.available = available;
        if !available && self.remote_sessions.active {
            // Access can disappear before Workspace updates host visibility.
            // Do not briefly mark the hidden local conversation as read.
            self.remote_sessions.active = false;
            self.set_surface_visible(false, cx);
        }
        cx.notify();
    }

    pub fn set_remote_session_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.close_navigator_overlay(cx);
        if active {
            self.clear_ssh_session_preview();
        }
        self.remote_sessions.active = active && self.remote_sessions.available;
        self.sessions.set_surface_visible(
            self.remote_sessions.surface_visible
                && !self.remote_sessions.active
                && self.ssh_sessions.preview.is_none(),
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
        let query = self
            .navigator_search
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let mut sessions = matching_sessions(source.agent_sessions(), &query);
        let total = sessions.len();
        let available_only = self.library.preferences.remote_sessions_available_only;
        if available_only {
            sessions.retain(|record| record.attachable);
        }
        let expanded = self.library.preferences.remote_sessions_expanded || !query.is_empty();
        let limit = remote_page_limit(
            &self.remote_sessions.page_query,
            &query,
            self.remote_sessions.visible_limit,
        );
        let shown = sessions.len().min(limit);
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .pt(px(6.0))
                .pb(px(3.0))
                .child(
                    Button::new("agent-remote-toggle", t!("agents.remote.title").to_string())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .flex_1()
                        .min_w(px(0.0))
                        .justify_start()
                        .disabled(!query.is_empty())
                        .icon(IconSource::from(if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        }))
                        .tooltip(
                            t!(if expanded {
                                "agents.remote.collapse"
                            } else {
                                "agents.remote.expand"
                            })
                            .to_string(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.library.preferences.remote_sessions_expanded =
                                !this.library.preferences.remote_sessions_expanded;
                            this.schedule_library_save(cx);
                            cx.notify();
                        })),
                )
                .child(
                    Badge::new(if available_only {
                        format!("{}/{total}", sessions.len())
                    } else {
                        total.to_string()
                    })
                    .variant(BadgeVariant::Secondary),
                )
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
        if !expanded {
            return list.into_any_element();
        }
        let sync_status = if source.agent_sessions_loading() {
            t!("agents.remote.refreshing_directory").to_string()
        } else if let Some(at) = source.agent_sessions_updated_at() {
            t!("agents.remote.directory_updated", time = at).to_string()
        } else {
            t!("agents.remote.directory_unchecked").to_string()
        };
        list = list.child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .child(sync_status),
        );
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(6.0))
                .child(
                    Button::new("agent-remote-all", t!("agents.remote.all").to_string())
                        .variant(if available_only {
                            ButtonVariant::Ghost
                        } else {
                            ButtonVariant::Secondary
                        })
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.library.preferences.remote_sessions_available_only = false;
                            this.remote_sessions.visible_limit = 20;
                            this.schedule_library_save(cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(
                        "agent-remote-available",
                        t!("agents.remote.available").to_string(),
                    )
                    .variant(if available_only {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(ButtonSize::Sm)
                    .tooltip(t!("agents.remote.available_hint").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.library.preferences.remote_sessions_available_only = true;
                        this.remote_sessions.visible_limit = 20;
                        this.schedule_library_save(cx);
                        cx.notify();
                    })),
                ),
        );
        if let Some(error) = source.agent_sessions_error() {
            list = list.child(
                Button::new(
                    "agent-remote-error",
                    t!("agents.remote.connection_unavailable").to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("cloud-off"))
                .tooltip(error.to_owned())
                .disabled(source.agent_sessions_loading())
                .on_click(
                    cx.listener(|_, _, _, cx| cx.emit(AgentConsoleEvent::RefreshRemoteSessions)),
                ),
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
        for record in sessions.iter().take(shown) {
            let coordinate = record.session.resource.clone();
            let identity = format!(
                "{} · {}",
                coordinate.authority.as_str(),
                coordinate.id.as_str()
            );
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
                .tooltip(move |_, cx| cx.new(|_| RemoteSessionTooltip(identity.clone())).into())
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
                                .child(remote_session_title(record)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(9.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(remote_session_description(
                                    record,
                                    source.agent_session_resume_available(&coordinate),
                                    source.agent_session_resume_pending(&coordinate),
                                )),
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
        if sessions.len() > shown {
            let page_query = query.clone();
            list = list.child(
                Button::new(
                    "agent-remote-more",
                    t!(
                        "agents.remote.show_more",
                        shown = shown,
                        total = sessions.len()
                    )
                    .to_string(),
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.remote_sessions.page_query = page_query.clone();
                    this.remote_sessions.visible_limit = shown.saturating_add(20);
                    cx.notify();
                })),
            );
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
            .relative()
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
        if self.tools.navigator_wide && self.tools.navigator_visible {
            root = root.child(self.render_navigator(cx));
        }
        if let Some(source) = self.remote_sessions.source.clone() {
            root = root.child(source.update(cx, |view, cx| view.render_agent_session(cx)));
        }
        if !self.tools.navigator_wide && self.tools.navigator_overlay {
            root = root.child(self.render_navigator_overlay(cx));
        }
        root.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        matching_sessions, remote_page_limit, remote_session_title, AgentConsoleView,
        AgentStreamEvent, FleetView, FreshnessState, SessionRecord, Uuid,
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

    // SDTEST-2746 / SDUC-559
    #[test]
    fn sdtest_2746_remote_directory_pages_searches_and_labels_lifecycle_records() {
        let mut directory = (0..119)
            .map(|i| {
                session(
                    &format!("session_bear_{i:04}_abcdef{i:04}"),
                    "open",
                    FreshnessState::Unknown,
                    i % 2 == 0,
                )
            })
            .collect::<Vec<_>>();
        directory[100].session.freshness.observed_at = EpochMillis::from_millis(1000);
        let matches = matching_sessions(&directory, "");
        assert_eq!(
            matches.len(),
            119,
            "observation-only entries stay reachable"
        );
        assert_eq!(matches[0].session.resource, directory[100].session.resource);
        assert_eq!(remote_page_limit("", "", 0), 20);
        assert_eq!(remote_page_limit("", "", 40), 40);
        assert_eq!(remote_page_limit("", "bear", 40), 20);
        assert_eq!(matching_sessions(&directory, "_0118_").len(), 1);
        assert_eq!(matching_sessions(&directory, "automonique").len(), 119);
        let label = remote_session_title(&directory[0]);
        assert!(label.contains("bear"));
        assert_eq!(matching_sessions(&directory, "session bear").len(), 119);
        assert_eq!(
            directory
                .iter()
                .map(remote_session_title)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            119
        );
        assert_ne!(label, "open");
        let titled = session("named", "Fix SSH routing", FreshnessState::Fresh, true);
        assert_eq!(remote_session_title(&titled), "Fix SSH routing");
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
