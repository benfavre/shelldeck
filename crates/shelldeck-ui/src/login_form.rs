//! Sign-in modal for Inklura Manage: email + password first, with browser/OIDC
//! alternatives disclosed on demand instead of competing with the primary path.
//!
//! Text entry mirrors `connection_form.rs`: a focused root captures `on_key_down`
//! and edits the active field. The password field renders masked.

use crate::scale::px;
use gpui::prelude::*;
use gpui::*;

use adabraka_ui::components::input::{Input, InputSize, InputState};

use crate::overlay::{window_backdrop, InputEscape};
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OtherLoginMethod {
    id: &'static str,
    label_key: &'static str,
    icon_path: Option<&'static str>,
    provider: Option<&'static str>,
}

const OTHER_METHODS_DEFAULT_EXPANDED: bool = false;
const OTHER_LOGIN_METHODS: [OtherLoginMethod; 4] = [
    OtherLoginMethod {
        id: "login-oidc-sso",
        label_key: "login.oidc_sso",
        icon_path: Some("images/logo-1clicpro.svg"),
        provider: Some("sso"),
    },
    OtherLoginMethod {
        id: "login-oidc-google",
        label_key: "login.oidc_google",
        icon_path: Some("images/logo-google.svg"),
        provider: Some("google"),
    },
    OtherLoginMethod {
        id: "login-oidc-github",
        label_key: "login.oidc_github",
        icon_path: Some("images/logo-github.svg"),
        provider: Some("github"),
    },
    OtherLoginMethod {
        id: "login-oidc-browser",
        label_key: "login.oidc_browser",
        icon_path: None,
        provider: None,
    },
];

#[derive(Debug, Clone)]
pub enum LoginFormEvent {
    /// Submit email + password for password login.
    SubmitPassword {
        email: String,
        password: String,
    },
    /// Start the browser OIDC flow. `None` = generic SSO; otherwise
    /// "google"/"github"/"sso".
    StartOidc(Option<String>),
    /// Open Manage's password-recovery page in the system browser.
    OpenForgotPassword,
    Cancel,
}

impl EventEmitter<LoginFormEvent> for LoginForm {}

pub struct LoginForm {
    email_state: Entity<InputState>,
    password_state: Entity<InputState>,
    device: String,
    server: String,
    /// A request is in flight (login or OIDC wait): disable inputs + show spinner.
    busy: bool,
    error: Option<String>,
    other_methods_expanded: bool,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

impl LoginForm {
    pub fn new(server: String, device: String, cx: &mut Context<Self>) -> Self {
        Self {
            email_state: cx.new(InputState::new),
            password_state: cx.new(InputState::new),
            device,
            server,
            busy: false,
            error: None,
            other_methods_expanded: OTHER_METHODS_DEFAULT_EXPANDED,
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

    fn field_value(state: &Entity<InputState>, cx: &Context<Self>) -> String {
        state.read(cx).content().to_string()
    }

    fn can_submit(&self, cx: &Context<Self>) -> bool {
        !self.busy
            && !Self::field_value(&self.email_state, cx).trim().is_empty()
            && !Self::field_value(&self.password_state, cx).is_empty()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            return;
        }
        let email = Self::field_value(&self.email_state, cx).trim().to_string();
        let password = Self::field_value(&self.password_state, cx);
        self.error = None;
        cx.emit(LoginFormEvent::SubmitPassword { email, password });
    }

    /// Only Escape needs to reach us — Input handles all typing keys, and
    /// `on_enter` on each field submits the form.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if event.keystroke.key == "escape" {
            cx.emit(LoginFormEvent::Cancel);
        }
    }

    /// A full-width OIDC provider button. `icon_path` puts a logo on the left
    /// (12px, muted color) — GPUI paints SVGs mono, so all provider logos
    /// share the current text_color regardless of their source fills.
    fn oidc_button(
        &self,
        id: &'static str,
        label: &str,
        icon_path: Option<&'static str>,
        provider: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let provider = provider.map(|p| p.to_string());
        let busy = self.busy;
        let mut row = div().flex().items_center().gap(px(8.0));
        if let Some(icon) = icon_path {
            row = row.child(
                svg()
                    .path(icon)
                    .size(px(14.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_primary()),
            );
        }
        row = row.child(label.to_string());

        let mut btn = div()
            .id(id)
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(14.0))
            .text_color(ShellDeckColors::text_primary())
            .flex()
            .items_center()
            .justify_center()
            .child(row);
        if busy {
            btn = btn.opacity(0.5);
        } else {
            btn = btn
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()));
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
            .w(px(420.0))
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
                        .items_center()
                        .gap(px(12.0))
                        // Inklura mark — hardcoded brand blue on a rounded
                        // portrait badge, since GPUI paints SVGs mono and the
                        // source is multi-color. Badge aspect ratio matches
                        // the source viewBox (78×118 → ~2:3).
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .flex_shrink_0()
                                .w(px(28.0))
                                .h(px(42.0))
                                .rounded(px(8.0))
                                .bg(rgb(0x146BFF))
                                .child(
                                    svg()
                                        .path("images/logo-inklura.svg")
                                        .w(px(28.0))
                                        .h(px(42.0))
                                        .text_color(gpui::white()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(17.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(t!("login.title").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(self.server.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("login-close")
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
                            cx.emit(LoginFormEvent::Cancel);
                        })),
                ),
        );

        // Real `Input` widgets — cursor, selection, undo, Enter submits.
        let labeled = |label: String, input: Input| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_muted())
                        .child(label),
                )
                .child(input)
        };
        let submit_on_enter = {
            let entity = cx.entity();
            move |_v: SharedString, cx: &mut App| {
                entity.update(cx, |this, cx| this.submit(cx));
            }
        };
        let email_input = Input::new(&self.email_state)
            .size(InputSize::Sm)
            .placeholder(t!("login.email_placeholder").to_string())
            .disabled(self.busy)
            .on_enter(submit_on_enter.clone());
        let password_input = Input::new(&self.password_state)
            .size(InputSize::Sm)
            .placeholder(t!("login.password_placeholder").to_string())
            .password(true)
            .disabled(self.busy)
            .on_enter(submit_on_enter);

        let mut forgot_password = div()
            .id("login-forgot-password")
            .text_size(px(12.0))
            .text_color(ShellDeckColors::primary())
            .child(t!("login.forgot_password").to_string());
        if self.busy {
            forgot_password = forgot_password.opacity(0.5);
        } else {
            forgot_password = forgot_password
                .cursor_pointer()
                .hover(|el| el.underline())
                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                    cx.emit(LoginFormEvent::OpenForgotPassword);
                }));
        }

        let password_field = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("login.password").to_string()),
                    )
                    .child(forgot_password),
            )
            .child(password_input);

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(20.0))
            .child(labeled(t!("login.email").to_string(), email_input))
            .child(password_field);

        if let Some(ref err) = self.error {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(err.clone()),
            );
        }

        // Primary "Se connecter" button.
        let can_submit = self.can_submit(cx);
        let submit_label = if self.busy {
            t!("login.submitting").to_string()
        } else {
            t!("login.submit").to_string()
        };
        let mut submit_btn = div()
            .id("login-submit")
            .w_full()
            .px(px(12.0))
            .py(px(9.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::primary())
            .text_size(px(14.0))
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

        let disclosure_icon = if self.other_methods_expanded {
            "chevron-down"
        } else {
            "chevron-right"
        };
        let mut disclosure = div()
            .id("login-other-methods")
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .py(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(ShellDeckColors::text_primary())
            .child(t!("login.other_methods").to_string())
            .child(
                svg()
                    .path(format!("icons/lucide/{disclosure_icon}.svg"))
                    .size(px(14.0))
                    .text_color(ShellDeckColors::text_muted()),
            );
        if self.busy {
            disclosure = disclosure.opacity(0.5);
        } else {
            disclosure = disclosure
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.other_methods_expanded = !this.other_methods_expanded;
                    cx.notify();
                }));
        }
        body = body.child(disclosure);

        if self.other_methods_expanded {
            let mut methods = div().flex().flex_col().gap(px(8.0));
            for method in OTHER_LOGIN_METHODS {
                methods = methods.child(self.oidc_button(
                    method.id,
                    t!(method.label_key).as_ref(),
                    method.icon_path,
                    method.provider,
                    cx,
                ));
            }
            methods = methods.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("login.device", device = self.device.clone()).to_string()),
            );
            body = body.child(methods);
        }

        card = card.child(body);

        // Full-screen focused overlay capturing keystrokes.
        window_backdrop("login-form-overlay", window.is_maximized())
            // Échap n'arrive pas comme touche quand un champ a le focus :
            // le contexte `Input` la lie à une action. Voir `crate::overlay`.
            .capture_action(cx.listener(|this, _: &InputEscape, _window, cx| {
                let _ = &this;
                if !this.busy {
                    cx.emit(LoginFormEvent::Cancel);
                }
            }))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key_down(event, cx);
            }))
            .flex()
            .justify_center()
            .items_center()
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::{OTHER_LOGIN_METHODS, OTHER_METHODS_DEFAULT_EXPANDED};

    #[test]
    fn alternative_login_methods_are_complete_and_collapsed_by_default() {
        assert!(!OTHER_METHODS_DEFAULT_EXPANDED);
        assert_eq!(OTHER_LOGIN_METHODS.len(), 4);
        assert_eq!(
            OTHER_LOGIN_METHODS.map(|method| method.provider),
            [Some("sso"), Some("google"), Some("github"), None]
        );
    }
}
