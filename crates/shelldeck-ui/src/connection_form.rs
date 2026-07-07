use crate::scale::px;
use gpui::prelude::*;
use gpui::*;

use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::components::toggle::Toggle;
use adabraka_ui::prelude::*;
use shelldeck_core::models::connection::Connection;
use uuid::Uuid;

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ConnectionFormEvent {
    Save(Connection),
    Cancel,
}

impl EventEmitter<ConnectionFormEvent> for ConnectionForm {}

/// Identifies which field a validation error belongs to, so the matching
/// `Input` renders its red-outline error state. Active-field tracking is no
/// longer needed — the `Input` widget owns its own focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    Hostname,
    Port,
    User,
}

pub struct ConnectionForm {
    /// Editing existing connection (Some) or creating new (None)
    editing_id: Option<Uuid>,
    alias_state: Entity<InputState>,
    hostname_state: Entity<InputState>,
    port_state: Entity<InputState>,
    user_state: Entity<InputState>,
    identity_file_state: Entity<InputState>,
    proxy_jump_state: Entity<InputState>,
    group_state: Entity<InputState>,
    forward_agent: bool,
    error: Option<String>,
    error_field: Option<FormField>,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

/// Create a new `Input` state entity with an optional initial value. We can't
/// use `InputState::set_value` in the constructor (it needs `&mut Window`),
/// so we bypass by writing the public `content` field directly.
fn new_input_state(cx: &mut Context<ConnectionForm>, initial: &str) -> Entity<InputState> {
    let initial = initial.to_string();
    cx.new(|cx| {
        let mut s = InputState::new(cx);
        if !initial.is_empty() {
            s.content = initial.into();
        }
        s
    })
}

impl ConnectionForm {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let default_user = whoami().unwrap_or_else(|| "root".to_string());
        Self {
            editing_id: None,
            alias_state: new_input_state(cx, ""),
            hostname_state: new_input_state(cx, ""),
            port_state: new_input_state(cx, "22"),
            user_state: new_input_state(cx, &default_user),
            identity_file_state: new_input_state(cx, ""),
            proxy_jump_state: new_input_state(cx, ""),
            group_state: new_input_state(cx, ""),
            forward_agent: false,
            error: None,
            error_field: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    pub fn from_connection(conn: &Connection, cx: &mut Context<Self>) -> Self {
        let identity_file = conn
            .identity_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        Self {
            editing_id: Some(conn.id),
            alias_state: new_input_state(cx, &conn.alias),
            hostname_state: new_input_state(cx, &conn.hostname),
            port_state: new_input_state(cx, &conn.port.to_string()),
            user_state: new_input_state(cx, &conn.user),
            identity_file_state: new_input_state(cx, &identity_file),
            proxy_jump_state: new_input_state(cx, conn.proxy_jump.as_deref().unwrap_or("")),
            group_state: new_input_state(cx, conn.group.as_deref().unwrap_or("")),
            forward_agent: conn.forward_agent,
            error: None,
            error_field: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        // Focus the sheet root so Escape reaches our `handle_key_down`. The
        // individual `Input` widgets take focus on click; we don't force the
        // hostname field to grab focus programmatically (would need `App`
        // access that isn't available here).
        self.focus_handle.focus(window);
    }

    fn field_value(state: &Entity<InputState>, cx: &Context<Self>) -> String {
        state.read(cx).content().to_string()
    }

    fn is_valid(&self, cx: &Context<Self>) -> bool {
        if Self::field_value(&self.hostname_state, cx).is_empty() {
            return false;
        }
        if Self::field_value(&self.user_state, cx).is_empty() {
            return false;
        }
        matches!(
            Self::field_value(&self.port_state, cx).parse::<u16>(),
            Ok(p) if p > 0
        )
    }

    /// Escape key on the sheet cancels the form. Enter is handled per-input
    /// via `on_enter` (submits the form). Text-editing keys are consumed by
    /// the focused `Input` widget.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            cx.emit(ConnectionFormEvent::Cancel);
        }
    }

    pub fn try_save(&mut self, cx: &mut Context<Self>) {
        match self.validate(cx) {
            Ok(conn) => {
                cx.emit(ConnectionFormEvent::Save(conn));
            }
            Err(msg) => {
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

    fn validate(&self, cx: &Context<Self>) -> Result<Connection, String> {
        let hostname = Self::field_value(&self.hostname_state, cx);
        let user = Self::field_value(&self.user_state, cx);
        let port_str = Self::field_value(&self.port_state, cx);
        let alias = Self::field_value(&self.alias_state, cx);
        let identity_file = Self::field_value(&self.identity_file_state, cx);
        let proxy_jump = Self::field_value(&self.proxy_jump_state, cx);
        let group = Self::field_value(&self.group_state, cx);

        if hostname.is_empty() {
            return Err("Hostname is required".to_string());
        }
        if user.is_empty() {
            return Err("Username is required".to_string());
        }
        let port: u16 = port_str
            .parse()
            .map_err(|_| "Port must be a valid number (1-65535)".to_string())?;
        if port == 0 {
            return Err("Port must be between 1 and 65535".to_string());
        }

        let alias = if alias.is_empty() {
            hostname.clone()
        } else {
            alias
        };

        let mut conn = Connection::new_manual(alias, hostname, user);
        conn.port = port;

        if let Some(id) = self.editing_id {
            conn.id = id;
        }

        if !identity_file.is_empty() {
            conn.identity_file = Some(std::path::PathBuf::from(identity_file));
        }
        if !proxy_jump.is_empty() {
            conn.proxy_jump = Some(proxy_jump);
        }
        if !group.is_empty() {
            conn.group = Some(group);
        }
        conn.forward_agent = self.forward_agent;

        Ok(conn)
    }

    /// A labeled `Input` widget. `error_field` variant triggers the red-error
    /// styling. Enter on any field tries to save.
    /// Open the OS file picker and, if the user picks a file, write its path
    /// into `identity_file_state`. Called by the "Browse…" button next to
    /// the Identity File input. Starts in the user's `~/.ssh/` if it exists,
    /// otherwise falls back to `$HOME`.
    fn pick_identity_file(&self, window: &mut Window, cx: &mut Context<Self>) {
        let starting_directory =
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|home| {
                    let ssh = home.join(".ssh");
                    if ssh.is_dir() {
                        ssh
                    } else {
                        home
                    }
                });
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select SSH key".into()),
            starting_directory,
        });
        let state = self.identity_file_state.clone();
        cx.spawn_in(window, async move |_this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let display = path.display().to_string();
            let _ = state.update(cx, |s, cx| {
                s.content = display.into();
                cx.notify();
            });
        })
        .detach();
    }

    /// Same layout as `render_field`, plus a "Browse…" button that opens the
    /// native file picker and writes the selected path back to the input.
    fn render_identity_file_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let input =
            self.render_field_input(None, &self.identity_file_state, "~/.ssh/id_ed25519", cx);
        let browse = div()
            .id("connection-form-identity-browse")
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .h(px(32.0))
            .px(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .child("Browse…")
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.pick_identity_file(window, cx);
            }));

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child("Identity File"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().flex_1().child(input))
                    .child(browse),
            )
    }

    /// Just the `Input` (no label + column wrapper) so it can be composed
    /// with a suffix / adjacent button as in `render_identity_file_field`.
    fn render_field_input(
        &self,
        field: Option<FormField>,
        state: &Entity<InputState>,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> Input {
        let has_error = field.is_some() && field == self.error_field;
        Input::new(state)
            .size(InputSize::Sm)
            .placeholder(placeholder)
            .error(has_error)
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |this, cx| this.try_save(cx));
                }
            })
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
    }

    fn render_field(
        &self,
        field: Option<FormField>,
        label: &'static str,
        state: &Entity<InputState>,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_error = field.is_some() && field == self.error_field;
        let input = Input::new(state)
            .size(InputSize::Sm)
            .placeholder(placeholder)
            .error(has_error)
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |this, cx| this.try_save(cx));
                }
            })
            // Any keystroke clears the previous error highlight.
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
                    .child(label),
            )
            .child(input)
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

        // "Forward Agent" — real adabraka `Toggle` (built-in animated switch,
        // themed via `theme.tokens.primary`).
        let toggle = Toggle::new("toggle-forward-agent")
            .checked(self.forward_agent)
            .on_click({
                let entity = cx.entity();
                move |checked, _window, cx| {
                    let checked = *checked;
                    entity.update(cx, |this, cx| {
                        this.forward_agent = checked;
                        cx.notify();
                    });
                }
            });

        let mut form_fields = div()
            .id("connection-form-fields")
            .flex()
            .flex_col()
            .flex_grow()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .gap(px(12.0))
            .p(px(20.0))
            // Row: Alias + Group
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(div().flex_grow().child(self.render_field(
                        None,
                        "Alias",
                        &self.alias_state,
                        "my-server",
                        cx,
                    )))
                    .child(div().w(px(140.0)).child(self.render_field(
                        None,
                        "Group",
                        &self.group_state,
                        "Production",
                        cx,
                    ))),
            )
            // Hostname
            .child(self.render_field(
                Some(FormField::Hostname),
                "Hostname",
                &self.hostname_state,
                "192.168.1.100 or example.com",
                cx,
            ))
            // Row: User + Port
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(div().flex_grow().child(self.render_field(
                        Some(FormField::User),
                        "User",
                        &self.user_state,
                        "deploy",
                        cx,
                    )))
                    .child(div().w(px(100.0)).child(self.render_field(
                        Some(FormField::Port),
                        "Port",
                        &self.port_state,
                        "22",
                        cx,
                    ))),
            )
            // Identity File — text input + native file picker button.
            .child(self.render_identity_file_field(cx))
            // ProxyJump
            .child(self.render_field(
                None,
                "ProxyJump",
                &self.proxy_jump_state,
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

        // Outer overlay: full-screen dimmed backdrop that closes on click, and
        // a right-anchored slide-in Sheet panel that holds the form. Feels
        // less blocking than the previous centered modal — the sidebar/list
        // behind stays partially visible.
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
            // Clicking on the dimmed area behind the sheet cancels the form.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _e, _window, cx| {
                    cx.emit(ConnectionFormEvent::Cancel);
                }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .flex_col()
                    .w(px(480.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_l_1()
                    .border_color(ShellDeckColors::border())
                    .shadow_xl()
                    .overflow_hidden()
                    // Clicks inside the sheet must not bubble to the backdrop
                    // (which would close the form).
                    .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                        cx.stop_propagation();
                    })
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
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child(
                                        svg()
                                            .path("images/close.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    )
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
                                let valid = self.is_valid(cx);
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
