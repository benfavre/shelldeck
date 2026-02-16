use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::port_forward::{ForwardDirection, PortForward};
use uuid::Uuid;

use crate::command_palette::fuzzy_match;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum PortForwardFormEvent {
    Save(PortForward),
    Cancel,
}

impl EventEmitter<PortForwardFormEvent> for PortForwardForm {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    Connection,
    Label,
    Direction,
    LocalHost,
    LocalPort,
    RemoteHost,
    RemotePort,
}

impl FormField {
    const ALL: &[FormField] = &[
        FormField::Connection,
        FormField::Label,
        FormField::Direction,
        FormField::LocalHost,
        FormField::LocalPort,
        FormField::RemoteHost,
        FormField::RemotePort,
    ];

    fn next(self) -> FormField {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }
}

pub struct PortForwardForm {
    editing_id: Option<Uuid>,
    connections: Vec<(Uuid, String, String)>,
    selected_connection_idx: usize,
    label: String,
    direction: ForwardDirection,
    local_host: String,
    local_port: String,
    remote_host: String,
    remote_port: String,
    error: Option<String>,
    error_field: Option<FormField>,
    active_field: Option<FormField>,
    focus_handle: FocusHandle,
    needs_focus: bool,
    // Dropdown state
    dropdown_open: bool,
    dropdown_query: String,
    dropdown_filtered: Vec<usize>,
    dropdown_selected: usize,
}

impl PortForwardForm {
    pub fn new(connections: Vec<(Uuid, String, String)>, cx: &mut Context<Self>) -> Self {
        let dropdown_filtered = (0..connections.len()).collect();
        Self {
            editing_id: None,
            connections,
            selected_connection_idx: 0,
            label: String::new(),
            direction: ForwardDirection::LocalToRemote,
            local_host: "127.0.0.1".to_string(),
            local_port: String::new(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: String::new(),
            error: None,
            error_field: None,
            active_field: Some(FormField::Label),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            dropdown_open: false,
            dropdown_query: String::new(),
            dropdown_filtered,
            dropdown_selected: 0,
        }
    }

    pub fn from_port_forward(
        forward: &PortForward,
        connections: Vec<(Uuid, String, String)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected_idx = connections
            .iter()
            .position(|(id, _, _)| *id == forward.connection_id)
            .unwrap_or(0);
        let dropdown_filtered = (0..connections.len()).collect();
        Self {
            editing_id: Some(forward.id),
            connections,
            selected_connection_idx: selected_idx,
            label: forward.label.clone().unwrap_or_default(),
            direction: forward.direction,
            local_host: forward.local_host.clone(),
            local_port: forward.local_port.to_string(),
            remote_host: forward.remote_host.clone(),
            remote_port: forward.remote_port.to_string(),
            error: None,
            error_field: None,
            active_field: Some(FormField::Label),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            dropdown_open: false,
            dropdown_query: String::new(),
            dropdown_filtered,
            dropdown_selected: 0,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn is_valid(&self) -> bool {
        if self.connections.is_empty() {
            return false;
        }
        let local_ok = matches!(self.local_port.parse::<u16>(), Ok(p) if p > 0);
        let remote_ok = matches!(self.remote_port.parse::<u16>(), Ok(p) if p > 0);
        local_ok && remote_ok
    }

    fn active_field_mut(&mut self) -> Option<&mut String> {
        self.active_field.map(move |f| match f {
            FormField::Connection => None,
            FormField::Label => Some(&mut self.label),
            FormField::Direction => None,
            FormField::LocalHost => Some(&mut self.local_host),
            FormField::LocalPort => Some(&mut self.local_port),
            FormField::RemoteHost => Some(&mut self.remote_host),
            FormField::RemotePort => Some(&mut self.remote_port),
        })?
    }

    fn cycle_direction(&mut self) {
        self.direction = match self.direction {
            ForwardDirection::LocalToRemote => ForwardDirection::RemoteToLocal,
            ForwardDirection::RemoteToLocal => ForwardDirection::Dynamic,
            ForwardDirection::Dynamic => ForwardDirection::LocalToRemote,
        };
    }

    fn update_dropdown_filter(&mut self) {
        let query_lower = self.dropdown_query.to_lowercase();
        if query_lower.is_empty() {
            self.dropdown_filtered = (0..self.connections.len()).collect();
        } else {
            self.dropdown_filtered = self
                .connections
                .iter()
                .enumerate()
                .filter(|(_, (_, name, host))| {
                    fuzzy_match(name, &query_lower) || fuzzy_match(host, &query_lower)
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.dropdown_selected = 0;
    }

    fn close_dropdown(&mut self) {
        self.dropdown_open = false;
        self.dropdown_query.clear();
        self.dropdown_filtered = (0..self.connections.len()).collect();
        self.dropdown_selected = 0;
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();

        // When dropdown is open, intercept all keys
        if self.dropdown_open {
            match key {
                "escape" => {
                    self.close_dropdown();
                    cx.notify();
                }
                "up" => {
                    if !self.dropdown_filtered.is_empty() && self.dropdown_selected > 0 {
                        self.dropdown_selected -= 1;
                    }
                    cx.notify();
                }
                "down" => {
                    if !self.dropdown_filtered.is_empty() {
                        self.dropdown_selected =
                            (self.dropdown_selected + 1).min(self.dropdown_filtered.len() - 1);
                    }
                    cx.notify();
                }
                "enter" => {
                    if let Some(&conn_idx) = self.dropdown_filtered.get(self.dropdown_selected) {
                        self.selected_connection_idx = conn_idx;
                    }
                    self.close_dropdown();
                    cx.notify();
                }
                "backspace" => {
                    self.dropdown_query.pop();
                    self.update_dropdown_filter();
                    cx.notify();
                }
                _ => {
                    if let Some(ref kc) = event.keystroke.key_char {
                        if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                            self.dropdown_query.push_str(kc);
                            self.update_dropdown_filter();
                            cx.notify();
                        }
                    } else if key.len() == 1
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.alt
                    {
                        self.dropdown_query.push_str(key);
                        self.update_dropdown_filter();
                        cx.notify();
                    }
                }
            }
            return;
        }

        match key {
            "escape" => {
                cx.emit(PortForwardFormEvent::Cancel);
            }
            "enter" => {
                if self.active_field == Some(FormField::Connection) {
                    self.dropdown_open = true;
                    self.dropdown_query.clear();
                    self.update_dropdown_filter();
                    cx.notify();
                    return;
                }
                self.try_save(cx);
            }
            "tab" => {
                if let Some(field) = self.active_field {
                    self.active_field = Some(field.next());
                    cx.notify();
                }
            }
            "backspace" => {
                if let Some(field) = self.active_field_mut() {
                    field.pop();
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                }
            }
            " " => {
                // Space cycles direction when that field is active
                if self.active_field == Some(FormField::Direction) {
                    self.cycle_direction();
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                    return;
                }
                // Space opens dropdown when connection field is active
                if self.active_field == Some(FormField::Connection) {
                    self.dropdown_open = true;
                    self.dropdown_query.clear();
                    self.update_dropdown_filter();
                    cx.notify();
                    return;
                }
                // Otherwise treat as normal char
                if let Some(field) = self.active_field_mut() {
                    field.push(' ');
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                }
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        if let Some(field) = self.active_field_mut() {
                            field.push_str(kc);
                            self.error = None;
                            self.error_field = None;
                            cx.notify();
                        }
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    if let Some(field) = self.active_field_mut() {
                        field.push_str(key);
                        self.error = None;
                        self.error_field = None;
                        cx.notify();
                    }
                }
            }
        }
    }

    fn try_save(&mut self, cx: &mut Context<Self>) {
        match self.validate() {
            Ok(forward) => {
                cx.emit(PortForwardFormEvent::Save(forward));
            }
            Err(msg) => {
                // Set error_field based on the validation error message
                if msg.contains("No connections") {
                    self.error_field = Some(FormField::Connection);
                } else if msg.contains("Local port") {
                    self.error_field = Some(FormField::LocalPort);
                } else if msg.contains("Remote port") {
                    self.error_field = Some(FormField::RemotePort);
                }
                self.error = Some(msg);
                cx.notify();
            }
        }
    }

    fn validate(&self) -> Result<PortForward, String> {
        if self.connections.is_empty() {
            return Err("No connections available".to_string());
        }
        let (connection_id, _, _) = &self.connections[self.selected_connection_idx];

        let local_port: u16 = self
            .local_port
            .parse()
            .map_err(|_| "Local port must be a number (1-65535)".to_string())?;
        if local_port == 0 {
            return Err("Local port must be between 1 and 65535".to_string());
        }

        let remote_port: u16 = self
            .remote_port
            .parse()
            .map_err(|_| "Remote port must be a number (1-65535)".to_string())?;
        if remote_port == 0 {
            return Err("Remote port must be between 1 and 65535".to_string());
        }

        let mut forward = match self.direction {
            ForwardDirection::LocalToRemote => {
                PortForward::new_local(*connection_id, local_port, &self.remote_host, remote_port)
            }
            ForwardDirection::RemoteToLocal => {
                PortForward::new_remote(*connection_id, remote_port, &self.local_host, local_port)
            }
            ForwardDirection::Dynamic => {
                let mut f = PortForward::new_local(
                    *connection_id,
                    local_port,
                    &self.remote_host,
                    remote_port,
                );
                f.direction = ForwardDirection::Dynamic;
                f
            }
        };

        // Override hosts from form (new_local/new_remote use defaults)
        forward.local_host = self.local_host.clone();
        forward.remote_host = self.remote_host.clone();

        if !self.label.is_empty() {
            forward.label = Some(self.label.clone());
        }

        if let Some(id) = self.editing_id {
            forward.id = id;
        }

        Ok(forward)
    }

    fn render_text_field(
        &self,
        field: FormField,
        label: &str,
        value: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_field == Some(field);
        let has_error = self.error_field == Some(field);

        let mut input_box = div()
            .id(ElementId::from(SharedString::from(format!(
                "pf-field-{field:?}"
            ))))
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_primary())
            .border_1()
            .text_size(px(13.0))
            .cursor_text()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.active_field = Some(field);
                cx.notify();
            }));

        if has_error {
            input_box = input_box.border_color(ShellDeckColors::error());
        } else if is_active {
            input_box = input_box.border_color(ShellDeckColors::primary());
        } else {
            input_box = input_box.border_color(ShellDeckColors::border());
        }

        if value.is_empty() {
            input_box = input_box.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(placeholder.to_string()),
            );
        } else {
            let mut text_el = div()
                .text_color(ShellDeckColors::text_primary())
                .flex()
                .child(value.to_string());

            if is_active {
                text_el =
                    text_el.child(div().w(px(1.0)).h(px(16.0)).bg(ShellDeckColors::primary()));
            }

            input_box = input_box.child(text_el);
        }

        if value.is_empty() && is_active {
            input_box = input_box.child(
                div()
                    .absolute()
                    .left(px(10.0))
                    .top(px(6.0))
                    .w(px(1.0))
                    .h(px(16.0))
                    .bg(ShellDeckColors::primary()),
            );
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
                    .child(label.to_string()),
            )
            .child(input_box)
    }

    fn render_direction_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Direction);
        let options = [
            (ForwardDirection::LocalToRemote, "L -> R"),
            (ForwardDirection::RemoteToLocal, "R -> L"),
            (ForwardDirection::Dynamic, "SOCKS"),
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
                    this.active_field = Some(FormField::Direction);
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

            chips = chips.child(chip.child(label));
        }

        let mut wrapper_border = ShellDeckColors::border();
        if is_active {
            wrapper_border = ShellDeckColors::primary();
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
                    .child("Direction"),
            )
            .child(
                div()
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(wrapper_border)
                    .child(chips),
            )
    }

    fn render_connection_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Connection);
        let has_error = self.error_field == Some(FormField::Connection);
        let conn_name = if self.connections.is_empty() {
            "(no connections)".to_string()
        } else {
            self.connections[self.selected_connection_idx].1.clone()
        };

        let mut border_color = ShellDeckColors::border();
        if has_error {
            border_color = ShellDeckColors::error();
        } else if is_active {
            border_color = ShellDeckColors::primary();
        }

        let mut wrapper = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child("Connection"),
            )
            .child(
                div()
                    .id("pf-connection-picker")
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(border_color)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.active_field = Some(FormField::Connection);
                        if this.dropdown_open {
                            this.close_dropdown();
                        } else {
                            this.dropdown_open = true;
                            this.dropdown_query.clear();
                            this.update_dropdown_filter();
                        }
                        this.error = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(conn_name),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.dropdown_open { "^" } else { "v" }),
                    ),
            );

        // Dropdown list
        if self.dropdown_open {
            // Search input area
            let search_area = div()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .text_size(px(12.0))
                .child(if self.dropdown_query.is_empty() {
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .flex()
                        .child("Type to filter...")
                        .child(div().w(px(1.0)).h(px(14.0)).bg(ShellDeckColors::primary()))
                } else {
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .flex()
                        .child(self.dropdown_query.clone())
                        .child(div().w(px(1.0)).h(px(14.0)).bg(ShellDeckColors::primary()))
                });

            let mut items_list = div()
                .id("pf-dropdown-list")
                .flex()
                .flex_col()
                .max_h(px(200.0))
                .overflow_y_scroll();

            if self.dropdown_filtered.is_empty() {
                items_list = items_list.child(
                    div()
                        .px(px(8.0))
                        .py(px(10.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("No matching connections"),
                );
            } else {
                for (fi, &conn_idx) in self.dropdown_filtered.iter().enumerate() {
                    let (_, ref name, ref hostname) = self.connections[conn_idx];
                    let is_highlighted = fi == self.dropdown_selected;

                    let mut item = div()
                        .id(ElementId::from(SharedString::from(format!("pf-dd-{}", fi))))
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .px(px(8.0))
                        .py(px(5.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_connection_idx = conn_idx;
                            this.close_dropdown();
                            cx.notify();
                        }));

                    if is_highlighted {
                        item = item.bg(ShellDeckColors::primary().opacity(0.15));
                    } else {
                        item = item.hover(|el| el.bg(ShellDeckColors::hover_bg()));
                    }

                    item = item
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(hostname.clone()),
                        );

                    items_list = items_list.child(item);
                }
            }

            wrapper = wrapper.child(
                div()
                    .w_full()
                    .mt(px(2.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .shadow_md()
                    .child(search_area)
                    .child(items_list),
            );
        }

        wrapper
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
            // Label
            .child(self.render_text_field(
                FormField::Label,
                "Label (optional)",
                &self.label.clone(),
                "My Web Server",
                cx,
            ))
            // Direction chips
            .child(self.render_direction_chips(cx))
            // Row: LocalHost + LocalPort
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(div().flex_grow().child(self.render_text_field(
                        FormField::LocalHost,
                        "Local Host",
                        &self.local_host.clone(),
                        "127.0.0.1",
                        cx,
                    )))
                    .child(div().w(px(120.0)).child(self.render_text_field(
                        FormField::LocalPort,
                        "Local Port",
                        &self.local_port.clone(),
                        "8080",
                        cx,
                    ))),
            )
            // Row: RemoteHost + RemotePort
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(div().flex_grow().child(self.render_text_field(
                        FormField::RemoteHost,
                        "Remote Host",
                        &self.remote_host.clone(),
                        "127.0.0.1",
                        cx,
                    )))
                    .child(div().w(px(120.0)).child(self.render_text_field(
                        FormField::RemotePort,
                        "Remote Port",
                        &self.remote_port.clone(),
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
                                        "Edit Port Forward"
                                    } else {
                                        "New Port Forward"
                                    }),
                            )
                            .child(
                                div()
                                    .id("close-pf-form-btn")
                                    .cursor_pointer()
                                    .text_size(px(16.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child("x")
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
                                        adabraka_ui::prelude::Button::new("cancel", "Cancel")
                                            .variant(adabraka_ui::prelude::ButtonVariant::Ghost),
                                    ),
                            )
                            .child({
                                let valid = self.is_valid();
                                let btn_label = if self.editing_id.is_some() {
                                    "Save Forward"
                                } else {
                                    "Create Forward"
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
