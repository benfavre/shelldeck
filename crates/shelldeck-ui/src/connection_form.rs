use gpui::prelude::*;
use gpui::*;

use adabraka_ui::prelude::*;
use shelldeck_core::models::connection::Connection;
use uuid::Uuid;

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum ConnectionFormEvent {
    Save(Connection),
    Cancel,
}

impl EventEmitter<ConnectionFormEvent> for ConnectionForm {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    Alias,
    Hostname,
    Port,
    User,
    IdentityFile,
    ProxyJump,
    Group,
}

impl FormField {
    const ALL: &[FormField] = &[
        FormField::Alias,
        FormField::Group,
        FormField::Hostname,
        FormField::User,
        FormField::Port,
        FormField::IdentityFile,
        FormField::ProxyJump,
    ];

    fn next(self) -> FormField {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }
}

pub struct ConnectionForm {
    /// Editing existing connection (Some) or creating new (None)
    editing_id: Option<Uuid>,
    alias: String,
    hostname: String,
    port: String,
    user: String,
    identity_file: String,
    proxy_jump: String,
    group: String,
    forward_agent: bool,
    error: Option<String>,
    error_field: Option<FormField>,
    active_field: Option<FormField>,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

impl ConnectionForm {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            editing_id: None,
            alias: String::new(),
            hostname: String::new(),
            port: "22".to_string(),
            user: whoami().unwrap_or_else(|| "root".to_string()),
            identity_file: String::new(),
            proxy_jump: String::new(),
            group: String::new(),
            forward_agent: false,
            error: None,
            error_field: None,
            active_field: Some(FormField::Hostname),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    pub fn from_connection(conn: &Connection, cx: &mut Context<Self>) -> Self {
        Self {
            editing_id: Some(conn.id),
            alias: conn.alias.clone(),
            hostname: conn.hostname.clone(),
            port: conn.port.to_string(),
            user: conn.user.clone(),
            identity_file: conn
                .identity_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            proxy_jump: conn.proxy_jump.clone().unwrap_or_default(),
            group: conn.group.clone().unwrap_or_default(),
            forward_agent: conn.forward_agent,
            error: None,
            error_field: None,
            active_field: Some(FormField::Hostname),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn is_valid(&self) -> bool {
        if self.hostname.is_empty() {
            return false;
        }
        if self.user.is_empty() {
            return false;
        }
        match self.port.parse::<u16>() {
            Ok(p) if p > 0 => true,
            _ => false,
        }
    }

    fn active_field_value(&self) -> Option<&str> {
        self.active_field.map(|f| match f {
            FormField::Alias => self.alias.as_str(),
            FormField::Hostname => self.hostname.as_str(),
            FormField::Port => self.port.as_str(),
            FormField::User => self.user.as_str(),
            FormField::IdentityFile => self.identity_file.as_str(),
            FormField::ProxyJump => self.proxy_jump.as_str(),
            FormField::Group => self.group.as_str(),
        })
    }

    fn active_field_mut(&mut self) -> Option<&mut String> {
        self.active_field.map(move |f| match f {
            FormField::Alias => &mut self.alias,
            FormField::Hostname => &mut self.hostname,
            FormField::Port => &mut self.port,
            FormField::User => &mut self.user,
            FormField::IdentityFile => &mut self.identity_file,
            FormField::ProxyJump => &mut self.proxy_jump,
            FormField::Group => &mut self.group,
        })
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                cx.emit(ConnectionFormEvent::Cancel);
            }
            "enter" => {
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
            Ok(conn) => {
                cx.emit(ConnectionFormEvent::Save(conn));
            }
            Err(msg) => {
                // Set error_field based on the validation error message
                if msg.contains("Hostname") {
                    self.error_field = Some(FormField::Hostname);
                } else if msg.contains("Username") {
                    self.error_field = Some(FormField::User);
                } else if msg.contains("Port") {
                    self.error_field = Some(FormField::Port);
                }
                self.error = Some(msg);
                cx.notify();
            }
        }
    }

    fn validate(&self) -> Result<Connection, String> {
        if self.hostname.is_empty() {
            return Err("Hostname is required".to_string());
        }
        if self.user.is_empty() {
            return Err("Username is required".to_string());
        }
        let port: u16 = self
            .port
            .parse()
            .map_err(|_| "Port must be a valid number (1-65535)".to_string())?;
        if port == 0 {
            return Err("Port must be between 1 and 65535".to_string());
        }

        let alias = if self.alias.is_empty() {
            self.hostname.clone()
        } else {
            self.alias.clone()
        };

        let mut conn = Connection::new_manual(alias, self.hostname.clone(), self.user.clone());
        conn.port = port;

        if let Some(id) = self.editing_id {
            conn.id = id;
        }

        if !self.identity_file.is_empty() {
            conn.identity_file = Some(std::path::PathBuf::from(&self.identity_file));
        }
        if !self.proxy_jump.is_empty() {
            conn.proxy_jump = Some(self.proxy_jump.clone());
        }
        if !self.group.is_empty() {
            conn.group = Some(self.group.clone());
        }
        conn.forward_agent = self.forward_agent;

        Ok(conn)
    }

    fn render_field(
        &self,
        field: FormField,
        label: &str,
        value: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_field == Some(field);

        let mut input_box = div()
            .id(ElementId::from(SharedString::from(format!("field-{field:?}"))))
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

        let has_error = self.error_field == Some(field);

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

            // Show cursor at the end when active
            if is_active {
                text_el = text_el.child(
                    div()
                        .w(px(1.0))
                        .h(px(16.0))
                        .bg(ShellDeckColors::primary()),
                );
            }

            input_box = input_box.child(text_el);
        }

        // Show cursor in empty field when active
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
}

impl Render for ConnectionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            self.focus_handle.focus(window);
        }
        let title = if self.editing_id.is_some() {
            "Edit Connection"
        } else {
            "New Connection"
        };

        let mut toggle = div()
            .id("toggle-forward-agent")
            .w(px(36.0))
            .h(px(20.0))
            .rounded_full()
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.forward_agent = !this.forward_agent;
                cx.notify();
            }));

        if self.forward_agent {
            toggle = toggle.bg(ShellDeckColors::primary());
        } else {
            toggle = toggle.bg(ShellDeckColors::toggle_off_bg());
        }

        let mut form_fields = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(20.0))
            // Row: Alias + Group
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_grow()
                            .child(self.render_field(FormField::Alias, "Alias", &self.alias.clone(), "my-server", cx)),
                    )
                    .child(
                        div()
                            .w(px(140.0))
                            .child(self.render_field(FormField::Group, "Group", &self.group.clone(), "Production", cx)),
                    ),
            )
            // Hostname
            .child(self.render_field(
                FormField::Hostname,
                "Hostname",
                &self.hostname.clone(),
                "192.168.1.100 or example.com",
                cx,
            ))
            // Row: User + Port
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_grow()
                            .child(self.render_field(FormField::User, "User", &self.user.clone(), "deploy", cx)),
                    )
                    .child(
                        div()
                            .w(px(100.0))
                            .child(self.render_field(FormField::Port, "Port", &self.port.clone(), "22", cx)),
                    ),
            )
            // Identity File
            .child(self.render_field(
                FormField::IdentityFile,
                "Identity File",
                &self.identity_file.clone(),
                "~/.ssh/id_ed25519",
                cx,
            ))
            // ProxyJump
            .child(self.render_field(
                FormField::ProxyJump,
                "ProxyJump",
                &self.proxy_jump.clone(),
                "bastion-host",
                cx,
            ))
            // Forward Agent toggle
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .child("Forward Agent"),
                    )
                    .child(toggle),
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

        // Outer overlay: full screen backdrop + centered form
        div()
            .id("connection-form-overlay")
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
                    .w(px(460.0))
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
                                    .child(title.to_string()),
                            )
                            .child(
                                div()
                                    .id("close-form-btn")
                                    .cursor_pointer()
                                    .text_size(px(16.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child("x")
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(ConnectionFormEvent::Cancel);
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
                                    .id("cancel-btn")
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(ConnectionFormEvent::Cancel);
                                    }))
                                    .child(
                                        Button::new("cancel", "Cancel")
                                            .variant(ButtonVariant::Ghost),
                                    ),
                            )
                            .child({
                                let valid = self.is_valid();
                                let mut save_btn = div()
                                    .id("save-btn")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.try_save(cx);
                                    }))
                                    .child(
                                        Button::new("save", "Save Connection")
                                            .variant(ButtonVariant::Default),
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

/// Get current username
fn whoami() -> Option<String> {
    std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("USERNAME").ok())
}
