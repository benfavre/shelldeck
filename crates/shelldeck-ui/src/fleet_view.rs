//! Native cockpit for the shared Automonique platform contract.
//!
//! This view is deliberately presentation-only. The workspace performs typed
//! client calls and returns attachments or leases; no provider or job runtime
//! exists in ShellDeck.

use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::display::badge::{Badge, BadgeVariant};
use adabraka_ui::prelude::Alert;
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::config::platform::{
    ActionResult, Attachment, ControlClaimResult, ControlLease, PaneStreamState, PlatformAction,
    PlatformActionPreview, PlatformCockpitState, PlatformRefresh, PlatformSnapshot, PlatformText,
    ResourceCoordinate, ResourceKind, ResourceRecord, SessionRecord,
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
}

impl EventEmitter<FleetViewEvent> for FleetView {}

pub struct FleetView {
    snapshot: Option<PlatformSnapshot>,
    cockpit: PlatformCockpitState,
    search_state: Entity<InputState>,
    search_query: String,
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
            cockpit: PlatformCockpitState::default(),
            search_state: cx.new(InputState::new),
            search_query: String::new(),
            selected_session: None,
            pending_action: None,
            refusal: None,
            loading: false,
            operation_busy: false,
            error: None,
        }
    }

    pub fn set_snapshot(&mut self, snapshot: PlatformSnapshot) {
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

    pub fn set_attached(&mut self, attachment: Attachment) {
        if let Some(snapshot) = self.snapshot.as_mut() {
            snapshot.view.track_attachment(&attachment);
        }
        self.selected_session = Some(attachment.session.id.as_str().to_owned());
        self.cockpit.attach(attachment);
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
        self.cockpit = PlatformCockpitState::default();
        self.search_query.clear();
        self.selected_session = None;
        self.pending_action = None;
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
                    this.selected_session = Some(select_id.clone());
                    this.cockpit.select(&session_coordinate);
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
                    this.selected_session = Some(coordinate.id.as_str().to_owned());
                    this.cockpit.select(&coordinate);
                    cx.notify();
                });
            });
            tabs = tabs.child(tab);
        }
        tabs
    }

    fn render_selected_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(pane) = self.cockpit.selected() else {
            return div().p(px(12.0));
        };
        let session = &pane.attachment.session;
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
        content
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
                    .child(self.render_resources(cx)),
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
                    .child(
                        div()
                            .px(px(10.0))
                            .py(px(8.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(self.render_session_search(cx)),
                    )
                    .child(self.render_pane_tabs(cx))
                    .child(self.render_selected_pane(cx))
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
            .child(content)
    }
}
