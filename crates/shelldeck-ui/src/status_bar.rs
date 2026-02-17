use gpui::*;

use crate::theme::ShellDeckColors;

pub struct StatusBar {
    pub active_connections: usize,
    pub active_forwards: usize,
    pub running_scripts: usize,
    pub notification: Option<String>,
    pub git_status: Option<String>,
    pub update_status: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            active_connections: 0,
            active_forwards: 0,
            running_scripts: 0,
            notification: None,
            git_status: None,
            update_status: None,
        }
    }

    pub fn set_counts(&mut self, connections: usize, forwards: usize, scripts: usize) {
        self.active_connections = connections;
        self.active_forwards = forwards;
        self.running_scripts = scripts;
    }

    pub fn set_notification(&mut self, msg: Option<String>) {
        self.notification = msg;
    }

    fn status_item(_icon: &str, count: usize, label: &str) -> impl IntoElement {
        div().flex().items_center().gap(px(4.0)).child(
            div()
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .child(format!("{} {}", count, label)),
        )
    }
}

impl Render for StatusBar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_shrink_0()
            .w_full()
            .h(px(28.0))
            .items_center()
            .justify_between()
            .px(px(12.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                // Left: status items
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(Self::status_item(
                        "server",
                        self.active_connections,
                        "connections",
                    ))
                    .child(Self::status_item(
                        "arrow-right-left",
                        self.active_forwards,
                        "forwards",
                    ))
                    .child(Self::status_item("play", self.running_scripts, "scripts")),
            )
            .child(
                // Center: git status
                {
                    let mut git_el = div().flex().items_center().gap(px(4.0));
                    if let Some(ref git) = self.git_status {
                        git_el = git_el.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::primary())
                                .child(git.clone()),
                        );
                    }
                    git_el
                },
            )
            .child(
                // Right: command palette hint + notification/version
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(ShellDeckColors::hint_bg())
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(if cfg!(target_os = "macos") {
                                        "\u{2318}\u{21E7}P"
                                    } else {
                                        "Ctrl+Shift+P"
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child("Command Palette"),
                            ),
                    )
                    .child({
                        let (text, color) = if let Some(ref update) = self.update_status {
                            (update.clone(), ShellDeckColors::primary())
                        } else if let Some(ref notif) = self.notification {
                            (notif.clone(), ShellDeckColors::text_muted())
                        } else {
                            (
                                format!("ShellDeck v{}", shelldeck_core::VERSION),
                                ShellDeckColors::text_muted(),
                            )
                        };
                        div().text_size(px(11.0)).text_color(color).child(text)
                    }),
            )
    }
}
