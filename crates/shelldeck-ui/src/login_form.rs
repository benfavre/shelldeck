//! Sign-in modal for Inklura Manage: email + password, plus one-click OIDC
//! (SSO / Google / GitHub) that hands off to the browser device-authorize flow.
//!
//! Text entry mirrors `connection_form.rs`: a focused root captures `on_key_down`
//! and edits the active field. The password field renders masked.

use gpui::prelude::*;
use gpui::*;
use crate::scale::px;

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum LoginFormEvent {
    /// Submit email + password for password login.
    SubmitPassword { email: String, password: String },
    /// Start the browser OIDC flow. `None` = generic SSO; otherwise
    /// "google"/"github"/"sso".
    StartOidc(Option<String>),
    Cancel,
}

impl EventEmitter<LoginFormEvent> for LoginForm {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Email,
    Password,
}

impl Field {
    fn next(self) -> Field {
        match self {
            Field::Email => Field::Password,
            Field::Password => Field::Email,
        }
    }
}

pub struct LoginForm {
    email: String,
    password: String,
    active_field: Field,
    device: String,
    server: String,
    /// A request is in flight (login or OIDC wait): disable inputs + show spinner.
    busy: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

impl LoginForm {
    pub fn new(server: String, device: String, cx: &mut Context<Self>) -> Self {
        Self {
            email: String::new(),
            password: String::new(),
            active_field: Field::Email,
            device,
            server,
            busy: false,
            error: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    /// Toggle the in-flight state (driven by the workspace around the network call).
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    /// Show a server/validation error under the form.
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.active_field {
            Field::Email => &mut self.email,
            Field::Password => &mut self.password,
        }
    }

    fn can_submit(&self) -> bool {
        !self.busy && !self.email.trim().is_empty() && !self.password.is_empty()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_submit() {
            return;
        }
        self.error = None;
        cx.emit(LoginFormEvent::SubmitPassword {
            email: self.email.trim().to_string(),
            password: self.password.clone(),
        });
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => cx.emit(LoginFormEvent::Cancel),
            "enter" => self.submit(cx),
            "tab" => {
                self.active_field = self.active_field.next();
                cx.notify();
            }
            "backspace" => {
                self.active_field_mut().pop();
                self.error = None;
                cx.notify();
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        self.active_field_mut().push_str(kc);
                        self.error = None;
                        cx.notify();
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    self.active_field_mut().push_str(key);
                    self.error = None;
                    cx.notify();
                }
            }
        }
    }

    fn render_field(
        &self,
        field: Field,
        label: &str,
        value: &str,
        placeholder: &str,
        mask: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_field == field;
        let display: String = if mask {
            "\u{2022}".repeat(value.chars().count())
        } else {
            value.to_string()
        };

        let mut input_box = div()
            .id(ElementId::from(SharedString::from(format!("login-field-{label}"))))
            .w_full()
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_primary())
            .border_1()
            .text_size(px(13.0))
            .cursor_text()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.active_field = field;
                cx.notify();
            }));

        if is_active {
            input_box = input_box.border_color(ShellDeckColors::primary());
        } else {
            input_box = input_box.border_color(ShellDeckColors::border());
        }

        if display.is_empty() {
            let mut ph = div()
                .relative()
                .text_color(ShellDeckColors::text_muted())
                .child(placeholder.to_string());
            if is_active {
                ph = ph.child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(1.0))
                        .w(px(1.0))
                        .h(px(15.0))
                        .bg(ShellDeckColors::primary()),
                );
            }
            input_box = input_box.child(ph);
        } else {
            let mut text_el = div()
                .flex()
                .text_color(ShellDeckColors::text_primary())
                .child(display);
            if is_active {
                text_el = text_el.child(div().w(px(1.0)).h(px(15.0)).bg(ShellDeckColors::primary()));
            }
            input_box = input_box.child(text_el);
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

    /// A full-width OIDC provider button.
    fn oidc_button(
        &self,
        id: &'static str,
        label: &str,
        provider: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let provider = provider.map(|p| p.to_string());
        let busy = self.busy;
        let mut btn = div()
            .id(id)
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(13.0))
            .text_color(ShellDeckColors::text_primary())
            .flex()
            .items_center()
            .justify_center()
            .child(label.to_string());
        if busy {
            btn = btn.opacity(0.5);
        } else {
            btn = btn.cursor_pointer().hover(|s| s.bg(ShellDeckColors::hover_bg()));
            btn = btn.on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                cx.emit(LoginFormEvent::StartOidc(provider.clone()));
            }));
        }
        btn
    }
}

impl Render for LoginForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            self.focus_handle.focus(window);
        }

        let mut card = div()
            .flex()
            .flex_col()
            .w(px(400.0))
            .bg(ShellDeckColors::bg_surface())
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .shadow_xl()
            .overflow_hidden();

        // Header
        card = card.child(
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
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child("Se connecter à Inklura Manage"),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(self.server.clone()),
                        ),
                )
                .child(
                    div()
                        .id("login-close")
                        .cursor_pointer()
                        .text_size(px(16.0))
                        .text_color(ShellDeckColors::text_muted())
                        .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                        .child("\u{00D7}")
                        .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                            cx.emit(LoginFormEvent::Cancel);
                        })),
                ),
        );

        // Body: password fields + submit, divider, OIDC buttons.
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(20.0))
            .child(self.render_field(Field::Email, "Email", &self.email.clone(), "vous@exemple.com", false, cx))
            .child(self.render_field(
                Field::Password,
                "Mot de passe",
                &self.password.clone(),
                "••••••••",
                true,
                cx,
            ));

        if let Some(ref err) = self.error {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(err.clone()),
            );
        }

        // Primary "Se connecter" button.
        let can_submit = self.can_submit();
        let submit_label = if self.busy { "Connexion…" } else { "Se connecter" };
        let mut submit_btn = div()
            .id("login-submit")
            .w_full()
            .px(px(12.0))
            .py(px(9.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::primary())
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white())
            .flex()
            .items_center()
            .justify_center()
            .child(submit_label.to_string());
        if can_submit {
            submit_btn = submit_btn
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx)));
        } else {
            submit_btn = submit_btn.opacity(0.5);
        }
        body = body.child(submit_btn);

        // Divider with "ou".
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .py(px(2.0))
                .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border()))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("ou"),
                )
                .child(div().flex_1().h(px(1.0)).bg(ShellDeckColors::border())),
        );

        body = body
            .child(self.oidc_button("login-oidc-sso", "Continuer avec SSO 1clic.pro", Some("sso"), cx))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(div().flex_1().child(self.oidc_button(
                        "login-oidc-google",
                        "Google",
                        Some("google"),
                        cx,
                    )))
                    .child(div().flex_1().child(self.oidc_button(
                        "login-oidc-github",
                        "GitHub",
                        Some("github"),
                        cx,
                    ))),
            )
            // Provider-less browser sign-in → manage password login page, which
            // round-trips back to authorize. For users with an existing manage
            // session or a password manager in the browser.
            .child(self.oidc_button(
                "login-oidc-browser",
                "Via le navigateur (mot de passe)",
                None,
                cx,
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("Appareil : {}", self.device)),
            );

        card = card.child(body);

        // Full-screen focused overlay capturing keystrokes.
        div()
            .id("login-form-overlay")
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
            .child(card)
    }
}
