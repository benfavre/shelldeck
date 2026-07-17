use crate::scale::px;
use adabraka_ui::components::combobox::Combobox;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::prelude::{Button, ButtonSize, ButtonVariant};
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::port_forward::{ForwardDirection, PortForward};
use uuid::Uuid;

use crate::connection_combobox::{build_connection_combobox, connection_idx_for_id};
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum PortForwardFormEvent {
    Save(PortForward),
    SuggestNameWithAi,
    Cancel,
}

impl EventEmitter<PortForwardFormEvent> for PortForwardForm {}

/// Identifies which field an error belongs to, so the matching `Input` (or
/// picker) renders red. Focus is owned by each `Input` widget individually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    Connection,
    LocalPort,
    RemotePort,
}

#[derive(Debug, Clone, Copy)]
enum ValidationError {
    NoConnections,
    LocalPortInvalid,
    LocalPortRange,
    RemotePortInvalid,
    RemotePortRange,
}

fn forward_form_error(err: ValidationError) -> String {
    match err {
        ValidationError::NoConnections => t!("forward_form.error.no_connections").to_string(),
        ValidationError::LocalPortInvalid => {
            t!("forward_form.error.local_port_invalid").to_string()
        }
        ValidationError::LocalPortRange => t!("forward_form.error.local_port_range").to_string(),
        ValidationError::RemotePortInvalid => {
            t!("forward_form.error.remote_port_invalid").to_string()
        }
        ValidationError::RemotePortRange => t!("forward_form.error.remote_port_range").to_string(),
    }
}

fn forward_form_error_field(err: ValidationError) -> FormField {
    match err {
        ValidationError::NoConnections => FormField::Connection,
        ValidationError::LocalPortInvalid | ValidationError::LocalPortRange => FormField::LocalPort,
        ValidationError::RemotePortInvalid | ValidationError::RemotePortRange => {
            FormField::RemotePort
        }
    }
}

pub struct PortForwardForm {
    editing_id: Option<Uuid>,
    connections: Vec<(Uuid, String, String)>,
    selected_connection_idx: usize,
    label_state: Entity<InputState>,
    direction: ForwardDirection,
    local_host_state: Entity<InputState>,
    local_port_state: Entity<InputState>,
    remote_host_state: Entity<InputState>,
    remote_port_state: Entity<InputState>,
    ai_enabled: bool,
    error: Option<String>,
    error_field: Option<FormField>,
    focus_handle: FocusHandle,
    needs_focus: bool,
    connection_combobox: Entity<Combobox<Uuid>>,
}

/// Create a new `InputState` entity with an optional initial value. `set_value`
/// requires a `Window` we don't have in constructors — write `content` directly.
fn new_input_state(cx: &mut Context<PortForwardForm>, initial: &str) -> Entity<InputState> {
    let initial = initial.to_string();
    cx.new(|cx| {
        let mut s = InputState::new(cx);
        if !initial.is_empty() {
            s.content = initial.into();
        }
        s
    })
}

impl PortForwardForm {
    fn init_connection_combobox(
        connections: &[(Uuid, String, String)],
        selected_idx: usize,
        cx: &mut Context<Self>,
    ) -> Entity<Combobox<Uuid>> {
        let parent = cx.entity();
        let placeholder = if connections.is_empty() {
            t!("forward_form.connection.none").to_string()
        } else {
            t!("forward_form.connection.select").to_string()
        };
        let (_state, combobox) = build_connection_combobox(
            connections,
            selected_idx,
            &placeholder,
            move |id, _window, cx| {
                parent.update(cx, |form, cx| {
                    if let Some(idx) = connection_idx_for_id(&form.connections, *id) {
                        form.selected_connection_idx = idx;
                        form.error = None;
                        form.error_field = None;
                    }
                    cx.notify();
                });
            },
            cx,
        );
        combobox
    }

    pub fn new(
        connections: Vec<(Uuid, String, String)>,
        ai_enabled: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let connection_combobox = Self::init_connection_combobox(&connections, 0, cx);
        Self {
            editing_id: None,
            connections,
            selected_connection_idx: 0,
            label_state: new_input_state(cx, ""),
            direction: ForwardDirection::LocalToRemote,
            local_host_state: new_input_state(cx, "127.0.0.1"),
            local_port_state: new_input_state(cx, ""),
            remote_host_state: new_input_state(cx, "127.0.0.1"),
            remote_port_state: new_input_state(cx, ""),
            ai_enabled,
            error: None,
            error_field: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            connection_combobox,
        }
    }

    pub fn from_port_forward(
        forward: &PortForward,
        connections: Vec<(Uuid, String, String)>,
        ai_enabled: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_idx = connections
            .iter()
            .position(|(id, _, _)| *id == forward.connection_id)
            .unwrap_or(0);
        let connection_combobox = Self::init_connection_combobox(&connections, selected_idx, cx);
        Self {
            editing_id: Some(forward.id),
            connections,
            selected_connection_idx: selected_idx,
            label_state: new_input_state(cx, forward.label.as_deref().unwrap_or("")),
            direction: forward.direction,
            local_host_state: new_input_state(cx, &forward.local_host),
            local_port_state: new_input_state(cx, &forward.local_port.to_string()),
            remote_host_state: new_input_state(cx, &forward.remote_host),
            remote_port_state: new_input_state(cx, &forward.remote_port.to_string()),
            ai_enabled,
            error: None,
            error_field: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            connection_combobox,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn field_value(state: &Entity<InputState>, cx: &App) -> String {
        state.read(cx).content().to_string()
    }

    pub fn ai_context_data(&self, cx: &App) -> serde_json::Value {
        let connection = self.connections.get(self.selected_connection_idx);
        serde_json::json!({
            "tunnel": {
                "current_label": Self::field_value(&self.label_state, cx),
                "direction": format!("{:?}", self.direction),
                "local_host": Self::field_value(&self.local_host_state, cx),
                "local_port": Self::field_value(&self.local_port_state, cx),
                "remote_host": Self::field_value(&self.remote_host_state, cx),
                "remote_port": Self::field_value(&self.remote_port_state, cx),
                "connection": connection.map(|(_, alias, hostname)| serde_json::json!({
                    "alias": alias,
                    "hostname": hostname,
                })),
            }
        })
    }

    pub fn apply_ai_name(&mut self, name: String, cx: &mut Context<Self>) {
        self.label_state
            .update(cx, |state, cx| state.replace_content(name, cx));
        cx.notify();
    }

    fn is_valid(&self, cx: &Context<Self>) -> bool {
        if self.connections.is_empty() {
            return false;
        }
        let local_ok = matches!(
            Self::field_value(&self.local_port_state, cx).parse::<u16>(),
            Ok(p) if p > 0
        );
        let remote_ok = matches!(
            Self::field_value(&self.remote_port_state, cx).parse::<u16>(),
            Ok(p) if p > 0
        );
        local_ok && remote_ok
    }

    /// Non-text keys — text is consumed by whichever `Input` widget has focus.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key.as_str() == "escape" {
            cx.emit(PortForwardFormEvent::Cancel);
        }
    }

    pub fn try_save(&mut self, cx: &mut Context<Self>) {
        match self.validate(cx) {
            Ok(forward) => {
                cx.emit(PortForwardFormEvent::Save(forward));
            }
            Err(err) => {
                self.error_field = Some(forward_form_error_field(err));
                self.error = Some(forward_form_error(err));
                cx.notify();
            }
        }
    }

    fn validate(&self, cx: &Context<Self>) -> Result<PortForward, ValidationError> {
        if self.connections.is_empty() {
            return Err(ValidationError::NoConnections);
        }
        let (connection_id, _, _) = &self.connections[self.selected_connection_idx];
        let label = Self::field_value(&self.label_state, cx);
        let local_host = Self::field_value(&self.local_host_state, cx);
        let local_port_str = Self::field_value(&self.local_port_state, cx);
        let remote_host = Self::field_value(&self.remote_host_state, cx);
        let remote_port_str = Self::field_value(&self.remote_port_state, cx);

        let local_port: u16 = local_port_str
            .parse()
            .map_err(|_| ValidationError::LocalPortInvalid)?;
        if local_port == 0 {
            return Err(ValidationError::LocalPortRange);
        }
        let remote_port: u16 = remote_port_str
            .parse()
            .map_err(|_| ValidationError::RemotePortInvalid)?;
        if remote_port == 0 {
            return Err(ValidationError::RemotePortRange);
        }

        let mut forward = match self.direction {
            ForwardDirection::LocalToRemote => {
                PortForward::new_local(*connection_id, local_port, &remote_host, remote_port)
            }
            ForwardDirection::RemoteToLocal => {
                PortForward::new_remote(*connection_id, remote_port, &local_host, local_port)
            }
            ForwardDirection::Dynamic => {
                let mut f =
                    PortForward::new_local(*connection_id, local_port, &remote_host, remote_port);
                f.direction = ForwardDirection::Dynamic;
                f
            }
        };
        forward.local_host = local_host;
        forward.remote_host = remote_host;
        if !label.is_empty() {
            forward.label = Some(label);
        }
        if let Some(id) = self.editing_id {
            forward.id = id;
        }
        Ok(forward)
    }

    fn render_text_field(
        &self,
        field: Option<FormField>,
        label: impl Into<SharedString>,
        state: &Entity<InputState>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_error = field.is_some() && field == self.error_field;
        let placeholder = placeholder.into();
        let input = Input::new(state)
            .size(InputSize::Sm)
            .placeholder(placeholder)
            .error(has_error)
            .on_change({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |this, cx| {
                        if this.error.is_some() {
                            this.error = None;
                            this.error_field = None;
                            cx.notify();
                        }
                    });
                }
            })
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |this, cx| this.try_save(cx));
                }
            });

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child(label.into()),
            )
            .child(input)
    }

    fn render_direction_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let options = [
            (
                ForwardDirection::LocalToRemote,
                t!("forward_form.direction.ltr").to_string(),
            ),
            (
                ForwardDirection::RemoteToLocal,
                t!("forward_form.direction.rtl").to_string(),
            ),
            (
                ForwardDirection::Dynamic,
                t!("forward_form.direction.socks").to_string(),
            ),
        ];

        let mut chips = div().flex().gap(px(6.0));

        for (dir, label) in options {
            let selected = self.direction == dir;
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "pf-dir-{label}"
                ))))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.direction = dir;
                    this.error = None;
                    cx.notify();
                }));

            if selected {
                chip = chip
                    .bg(ShellDeckColors::primary().opacity(0.2))
                    .text_color(ShellDeckColors::primary())
                    .border_1()
                    .border_color(ShellDeckColors::primary());
            } else {
                chip = chip
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_muted())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::text_muted()));
            }

            chips = chips.child(chip.child(label.clone()));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("forward_form.field.direction").to_string()),
            )
            .child(
                div()
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .child(chips),
            )
    }

    fn render_connection_picker(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let has_error = self.error_field == Some(FormField::Connection);

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("forward_form.field.connection").to_string()),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .when(has_error, |el| {
                        el.rounded(px(6.0))
                            .border_1()
                            .border_color(ShellDeckColors::error())
                    })
                    .child(self.connection_combobox.clone()),
            )
    }
}

impl Render for PortForwardForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            self.focus_handle.focus(window);
        }
        let mut form_fields = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(20.0))
            // Connection picker
            .child(self.render_connection_picker(cx))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(8.0))
                    .child(div().flex_1().min_w(px(0.0)).child(self.render_text_field(
                        None,
                        t!("forward_form.field.label").to_string(),
                        &self.label_state,
                        t!("forward_form.field.label_placeholder").to_string(),
                        cx,
                    )))
                    .when(self.ai_enabled, |row| {
                        row.child(
                            Button::new("tunnel-ai-name", t!("ai.naming.action").to_string())
                                .variant(ButtonVariant::Ai)
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("sparkles"))
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(PortForwardFormEvent::SuggestNameWithAi);
                                })),
                        )
                    }),
            )
            .child(self.render_direction_chips(cx))
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(div().flex_grow().child(self.render_text_field(
                        None,
                        t!("forward_form.field.local_host").to_string(),
                        &self.local_host_state,
                        "127.0.0.1",
                        cx,
                    )))
                    .child(div().w(px(120.0)).child(self.render_text_field(
                        Some(FormField::LocalPort),
                        t!("forward_form.field.local_port").to_string(),
                        &self.local_port_state,
                        "8080",
                        cx,
                    ))),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(div().flex_grow().child(self.render_text_field(
                        None,
                        t!("forward_form.field.remote_host").to_string(),
                        &self.remote_host_state,
                        "127.0.0.1",
                        cx,
                    )))
                    .child(div().w(px(120.0)).child(self.render_text_field(
                        Some(FormField::RemotePort),
                        t!("forward_form.field.remote_port").to_string(),
                        &self.remote_port_state,
                        "80",
                        cx,
                    ))),
            );

        // Error message
        if let Some(ref error) = self.error {
            form_fields = form_fields.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }

        div()
            .id("port-forward-form-overlay")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key_down(event, cx);
            }))
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(ShellDeckColors::backdrop())
            .flex()
            .justify_center()
            .items_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(480.0))
                    .bg(ShellDeckColors::bg_surface())
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .shadow_xl()
                    .overflow_hidden()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(20.0))
                            .py(px(16.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(if self.editing_id.is_some() {
                                        t!("forward_form.title.edit").to_string()
                                    } else {
                                        t!("forward_form.title.new").to_string()
                                    }),
                            )
                            .child(
                                div()
                                    .id("close-pf-form-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child(
                                        svg()
                                            .path("icons/lucide/x.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    )
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(PortForwardFormEvent::Cancel);
                                    })),
                            ),
                    )
                    // Form fields
                    .child(form_fields)
                    // Footer
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(8.0))
                            .px(px(20.0))
                            .py(px(16.0))
                            .border_t_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .id("pf-cancel-btn")
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(PortForwardFormEvent::Cancel);
                                    }))
                                    .child(
                                        adabraka_ui::prelude::Button::new(
                                            "cancel",
                                            t!("scripts.cancel").to_string(),
                                        )
                                        .variant(adabraka_ui::prelude::ButtonVariant::Ghost),
                                    ),
                            )
                            .child({
                                let valid = self.is_valid(cx);
                                let btn_label = if self.editing_id.is_some() {
                                    t!("forward_form.save.edit").to_string()
                                } else {
                                    t!("forward_form.save.create").to_string()
                                };
                                let mut save_btn = div()
                                    .id("pf-save-btn")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.try_save(cx);
                                    }))
                                    .child(
                                        adabraka_ui::prelude::Button::new("save", btn_label)
                                            .variant(adabraka_ui::prelude::ButtonVariant::Default),
                                    );
                                if valid {
                                    save_btn = save_btn.cursor_pointer();
                                } else {
                                    save_btn = save_btn.opacity(0.5);
                                }
                                save_btn
                            }),
                    ),
            )
    }
}
