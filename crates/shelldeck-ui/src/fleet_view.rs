//! Native cockpit for the shared Automonique platform contract.
//!
//! This view is deliberately presentation-only. The workspace performs typed
//! client calls and returns attachments or leases; no provider or job runtime
//! exists in ShellDeck.

use std::collections::{BTreeMap, BTreeSet};

use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::display::badge::{Badge, BadgeVariant};
use adabraka_ui::prelude::Alert;
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::config::platform::{
    Attachment, ControlLease, PlatformSnapshot, ResourceCoordinate, ResourceKind, ResourceRecord,
    SessionRecord,
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
}

impl EventEmitter<FleetViewEvent> for FleetView {}

pub struct FleetView {
    snapshot: Option<PlatformSnapshot>,
    attachments: BTreeSet<String>,
    leases: BTreeMap<String, ControlLease>,
    selected_session: Option<String>,
    loading: bool,
    error: Option<String>,
}

impl FleetView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: None,
            attachments: BTreeSet::new(),
            leases: BTreeMap::new(),
            selected_session: None,
            loading: false,
            error: None,
        }
    }

    pub fn set_snapshot(&mut self, snapshot: PlatformSnapshot) {
        self.snapshot = Some(snapshot);
        self.loading = false;
        self.error = None;
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.loading = false;
    }

    pub fn set_attached(&mut self, attachment: Attachment) {
        self.attachments.insert(resource_key(&attachment.session));
        self.error = None;
    }

    pub fn set_detached(&mut self, session: &ResourceCoordinate) {
        let key = resource_key(session);
        self.attachments.remove(&key);
        self.leases.remove(&key);
        self.error = None;
    }

    pub fn set_control_lease(&mut self, lease: ControlLease) {
        self.leases.insert(resource_key(&lease.session), lease);
        self.error = None;
    }

    pub fn set_control_released(&mut self, session: &ResourceCoordinate) {
        self.leases.remove(&resource_key(session));
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
                    snapshot
                        .resources
                        .iter()
                        .filter(|resource| resource.resource.kind == ResourceKind::Receipt)
                        .count(),
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
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |_this, cx| cx.emit(FleetViewEvent::Refresh));
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

    fn render_resource(resource: &ResourceRecord) -> impl IntoElement {
        let freshness = resource.freshness.state.as_str();
        let freshness_color = match freshness {
            "fresh" => ShellDeckColors::success(),
            "stale" => ShellDeckColors::warning(),
            _ => ShellDeckColors::text_muted(),
        };
        div()
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
            )
    }

    fn render_resources(&self) -> impl IntoElement {
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
                list = list.child(Self::render_resource(resource));
            }
        }
        list
    }

    fn render_session(&self, session: &SessionRecord, cx: &mut Context<Self>) -> impl IntoElement {
        let coordinate = &session.session.resource;
        let key = resource_key(coordinate);
        let attached = self.attachments.contains(&key);
        let lease = self.leases.get(&key);
        let selected = self.selected_session.as_deref() == Some(coordinate.id.as_str());
        let entity = cx.entity();
        let select_id = coordinate.id.as_str().to_owned();
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
                    this.selected_session = Some(select_id.clone());
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
        let sessions = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.sessions.as_slice())
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
                    .child(self.render_resources()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(Self::section_header(
                        t!("fleet.sessions.section").to_string(),
                        session_count,
                    ))
                    .child(self.render_sessions(cx)),
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
        div()
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_header(cx))
            .child(content)
    }
}
