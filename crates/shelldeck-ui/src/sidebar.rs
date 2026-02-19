use gpui::prelude::*;
use gpui::*;

use adabraka_ui::prelude::*;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use uuid::Uuid;

use crate::command_palette::fuzzy_match;
use crate::theme::ShellDeckColors;

/// Returns indices of matched characters in haystack for a fuzzy needle.
fn fuzzy_match_indices(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    let haystack_lower: Vec<char> = haystack.to_lowercase().chars().collect();
    let needle_lower: Vec<char> = needle.to_lowercase().chars().collect();
    let mut indices = Vec::with_capacity(needle_lower.len());
    let mut hi = 0;
    for &nc in &needle_lower {
        loop {
            if hi >= haystack_lower.len() {
                return None;
            }
            if haystack_lower[hi] == nc {
                indices.push(hi);
                hi += 1;
                break;
            }
            hi += 1;
        }
    }
    Some(indices)
}

/// Render text with highlighted character indices.
fn render_highlighted_text(
    text: &str,
    matched_indices: &[usize],
    base_size: f32,
    base_color: Hsla,
    highlight_color: Hsla,
) -> Div {
    let chars: Vec<char> = text.chars().collect();
    let mut container = div()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap();
    let mut i = 0;
    while i < chars.len() {
        let is_match = matched_indices.contains(&i);
        // Batch consecutive chars of the same highlight state
        let start = i;
        while i < chars.len() && matched_indices.contains(&i) == is_match {
            i += 1;
        }
        let segment: String = chars[start..i].iter().collect();
        if is_match {
            container = container.child(
                div()
                    .text_color(highlight_color)
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(base_size))
                    .child(segment),
            );
        } else {
            container = container.child(
                div()
                    .text_color(base_color)
                    .text_size(px(base_size))
                    .child(segment),
            );
        }
    }
    container
}

/// Navigation sections in the sidebar
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    Connections,
    Scripts,
    PortForwards,
    ServerSync,
    Sites,
    FileEditor,
    Settings,
}

/// Events emitted by the sidebar
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    ConnectionSelected(Uuid),
    ConnectionConnect(Uuid),
    ConnectionEdit(Uuid),
    ConnectionDelete(Uuid),
    AddConnection,
    SectionChanged(SidebarSection),
    QuickConnect,
    WidthChanged(f32),
}

pub struct SidebarView {
    connections: Vec<Connection>,
    selected_connection: Option<Uuid>,
    active_section: SidebarSection,
    collapsed: bool,
    width: f32,
    /// Whether the user is currently dragging the resize handle.
    resizing: bool,
    /// Host search query
    search_query: String,
    search_focused: bool,
    focus_handle: FocusHandle,
}

impl SidebarView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            connections: Vec::new(),
            selected_connection: None,
            active_section: SidebarSection::Connections,
            collapsed: false,
            width: 260.0,
            resizing: false,
            search_query: String::new(),
            search_focused: false,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn is_resizing(&self) -> bool {
        self.resizing
    }

    pub fn set_width(&mut self, width: f32) {
        self.width = width.clamp(180.0, 400.0);
    }

    pub fn stop_resizing(&mut self) {
        self.resizing = false;
    }

    pub fn set_connections(&mut self, connections: Vec<Connection>) {
        self.connections = connections;
    }

    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn render_nav_item(
        &self,
        section: SidebarSection,
        label: &str,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_section == section;

        div()
            .id(ElementId::from(SharedString::from(format!(
                "nav-{section:?}"
            ))))
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .overflow_hidden()
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                this.active_section = section;
                cx.emit(SidebarEvent::SectionChanged(section));
                cx.notify();
            }))
            .when(is_active, |el| {
                el.bg(ShellDeckColors::primary().opacity(0.15))
                    .text_color(ShellDeckColors::primary())
            })
            .when(!is_active, |el| {
                el.text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            })
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .min_w(px(0.0))
                    .child(label.to_string()),
            )
            .when_some(count, |el, count| {
                el.child(
                    div()
                        .text_size(px(11.0))
                        .px(px(6.0))
                        .py(px(1.0))
                        .rounded(px(10.0))
                        .bg(ShellDeckColors::badge_bg())
                        .flex_shrink_0()
                        .child(count.to_string()),
                )
            })
    }

    fn render_section_header(label: &str) -> impl IntoElement {
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .w_full()
            .overflow_hidden()
            .px(px(12.0))
            .py(px(4.0))
            .mt(px(8.0))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .child(label.to_uppercase()),
            )
    }

    fn conn_matches_search(&self, conn: &Connection) -> bool {
        if self.search_query.is_empty() {
            return true;
        }
        let q = &self.search_query;
        fuzzy_match(conn.display_name(), q)
            || fuzzy_match(&conn.hostname, q)
            || fuzzy_match(&conn.user, q)
            || conn.group.as_deref().is_some_and(|g| fuzzy_match(g, q))
    }

    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut input_box = div()
            .id("sidebar-search-input")
            .flex()
            .items_center()
            .gap(px(6.0))
            .w_full()
            .px(px(8.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_primary())
            .border_1()
            .cursor_text()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.search_focused = true;
                cx.notify();
            }));

        if self.search_focused {
            input_box = input_box.border_color(ShellDeckColors::primary());
        } else {
            input_box = input_box.border_color(ShellDeckColors::border());
        }

        // Search icon
        input_box = input_box.child(
            div()
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .flex_shrink_0()
                .child("/"),
        );

        if self.search_query.is_empty() {
            input_box = input_box.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .flex_grow()
                    .child("Filter hosts..."),
            );
        } else {
            input_box = input_box.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .flex_grow()
                    .flex()
                    .child(self.search_query.clone())
                    .when(self.search_focused, |el| {
                        el.child(div().w(px(1.0)).h(px(14.0)).bg(ShellDeckColors::primary()))
                    }),
            );
            // Clear button
            input_box = input_box.child(
                div()
                    .id("sidebar-search-clear")
                    .cursor_pointer()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                    .flex_shrink_0()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.search_query.clear();
                        cx.notify();
                    }))
                    .child("x"),
            );
        }

        div()
            .flex_shrink_0()
            .px(px(8.0))
            .py(px(6.0))
            .child(input_box)
    }

    fn render_connection_item_highlighted(
        &self,
        connection: &Connection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_connection == Some(connection.id);
        let conn_id = connection.id;
        let status_color = match &connection.status {
            ConnectionStatus::Connected => ShellDeckColors::status_connected(),
            ConnectionStatus::Connecting => ShellDeckColors::warning(),
            ConnectionStatus::Disconnected => ShellDeckColors::status_disconnected(),
            ConnectionStatus::Error(_) => ShellDeckColors::status_error(),
        };

        let group_name = SharedString::from(format!("conn-group-{}", conn_id));

        // Compute highlight indices
        let name = connection.display_name().to_string();
        let conn_str = connection.connection_string();
        let name_indices = if !self.search_query.is_empty() {
            fuzzy_match_indices(&name, &self.search_query).unwrap_or_default()
        } else {
            vec![]
        };
        let conn_str_indices = if !self.search_query.is_empty() {
            fuzzy_match_indices(&conn_str, &self.search_query).unwrap_or_default()
        } else {
            vec![]
        };

        // Hover-reveal action buttons
        let action_buttons = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .flex_shrink_0()
            .opacity(0.0)
            .group_hover(group_name.clone(), |el| el.opacity(1.0))
            .child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "conn-connect-{}",
                        conn_id
                    ))))
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(ShellDeckColors::success().opacity(0.15))
                            .text_color(ShellDeckColors::success())
                    })
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SidebarEvent::ConnectionConnect(conn_id));
                    }))
                    .child("SSH"),
            )
            .child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "conn-edit-{}",
                        conn_id
                    ))))
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(ShellDeckColors::primary().opacity(0.15))
                            .text_color(ShellDeckColors::primary())
                    })
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SidebarEvent::ConnectionEdit(conn_id));
                    }))
                    .child("Edit"),
            )
            .child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "conn-del-{}",
                        conn_id
                    ))))
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(ShellDeckColors::error().opacity(0.15))
                            .text_color(ShellDeckColors::error())
                    })
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SidebarEvent::ConnectionDelete(conn_id));
                    }))
                    .child("Del"),
            );

        let mut row = div()
            .group(group_name)
            .flex()
            .flex_shrink_0()
            .items_center()
            .w_full()
            .overflow_hidden()
            .rounded(px(4.0))
            .when(is_selected, |el| el.bg(ShellDeckColors::selected_bg()))
            .hover(|el| el.bg(ShellDeckColors::hover_bg()));

        // Name/conn string with highlighting
        let name_el = if !name_indices.is_empty() {
            render_highlighted_text(
                &name,
                &name_indices,
                13.0,
                ShellDeckColors::text_primary(),
                ShellDeckColors::primary(),
            )
            .font_weight(FontWeight::MEDIUM)
        } else {
            div()
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .font_weight(FontWeight::MEDIUM)
                .child(name)
        };

        let conn_str_el = if !conn_str_indices.is_empty() {
            render_highlighted_text(
                &conn_str,
                &conn_str_indices,
                11.0,
                ShellDeckColors::text_muted(),
                ShellDeckColors::primary(),
            )
        } else {
            div()
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .child(conn_str)
        };

        let content = div()
            .id(ElementId::from(SharedString::from(format!(
                "conn-{}",
                conn_id
            ))))
            .flex()
            .items_center()
            .gap(px(8.0))
            .flex_grow()
            .min_w(px(0.0))
            .overflow_hidden()
            .px(px(12.0))
            .py(px(5.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                this.selected_connection = Some(conn_id);
                cx.emit(SidebarEvent::ConnectionSelected(conn_id));
                cx.notify();
            }))
            .child(
                div()
                    .w(px(8.0))
                    .h(px(8.0))
                    .rounded_full()
                    .bg(status_color)
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .flex_grow()
                    .child(name_el)
                    .child(conn_str_el),
            );

        row = row.child(content);
        row = row.child(action_buttons);
        row
    }

    fn handle_search_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        match key {
            "escape" => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                } else {
                    self.search_focused = false;
                }
                cx.notify();
                return;
            }
            "backspace" => {
                self.search_query.pop();
                cx.notify();
                return;
            }
            _ => {}
        }

        // Ctrl+V paste
        if key == "v" && mods.secondary() {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    // Only take first line, strip whitespace
                    let clean: String = text.lines().next().unwrap_or("").trim().to_string();
                    self.search_query.push_str(&clean);
                    cx.notify();
                }
            }
            return;
        }

        // Printable characters
        if let Some(ref kc) = event.keystroke.key_char {
            if !mods.control && !mods.alt {
                self.search_query.push_str(kc);
                cx.notify();
                return;
            }
        }

        if key.len() == 1 && !mods.control && !mods.alt {
            self.search_query.push_str(key);
            cx.notify();
        }
    }
}

impl EventEmitter<SidebarEvent> for SidebarView {}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.collapsed {
            return div().w(px(0.0)).h_full().id("sidebar-collapsed");
        }

        let search_focused = self.search_focused;

        // Filter connections by search query
        let filtered: Vec<&Connection> = self
            .connections
            .iter()
            .filter(|c| self.conn_matches_search(c))
            .collect();

        // Group filtered connections by group
        let mut grouped: std::collections::BTreeMap<String, Vec<&Connection>> =
            std::collections::BTreeMap::new();
        let mut ungrouped: Vec<&Connection> = Vec::new();

        for conn in &filtered {
            if let Some(ref group) = conn.group {
                grouped.entry(group.clone()).or_default().push(conn);
            } else {
                ungrouped.push(conn);
            }
        }

        let connected_count = self
            .connections
            .iter()
            .filter(|c| matches!(c.status, ConnectionStatus::Connected))
            .count();

        // Logo/title (pinned at top)
        let logo = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(10.0))
            .w_full()
            .overflow_hidden()
            .px(px(14.0))
            .py(px(14.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            // Icon badge: rounded square with terminal prompt glyph
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(30.0))
                    .h(px(30.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::primary())
                    .shadow_sm()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(gpui::white())
                            .child(">_"),
                    ),
            )
            // Wordmark: "Shell" in primary text, "Deck" in brand color
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child("Shell"),
                    )
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::primary())
                            .child("Deck"),
                    ),
            );

        // Navigation tabs (pinned at top)
        let nav = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(8.0))
            .child(self.render_nav_item(
                SidebarSection::Connections,
                "Connections",
                Some(connected_count),
                cx,
            ))
            .child(self.render_nav_item(SidebarSection::Scripts, "Scripts", None, cx))
            .child(self.render_nav_item(SidebarSection::PortForwards, "Port Forwards", None, cx))
            .child(self.render_nav_item(SidebarSection::ServerSync, "Server Sync", None, cx))
            .child(self.render_nav_item(SidebarSection::Sites, "Sites", None, cx))
            .child(self.render_nav_item(SidebarSection::FileEditor, "Editor", None, cx))
            .child(self.render_nav_item(SidebarSection::Settings, "Settings", None, cx));

        // Scrollable host list (fills remaining space)
        let mut host_list = div()
            .flex()
            .flex_col()
            .flex_grow()
            .min_h(px(0.0))
            .id("sidebar-host-list")
            .overflow_y_scroll()
            .child(Self::render_section_header("Hosts"))
            .child(self.render_search_bar(cx));

        // Ungrouped connections (with highlights)
        for conn in &ungrouped {
            host_list = host_list.child(self.render_connection_item_highlighted(conn, cx));
        }

        // Grouped connections (with highlights)
        for (group_name, conns) in &grouped {
            host_list = host_list.child(Self::render_section_header(group_name));
            for conn in conns {
                host_list = host_list.child(self.render_connection_item_highlighted(conn, cx));
            }
        }

        // "No matches" message when filtering yields nothing
        if !self.search_query.is_empty() && filtered.is_empty() {
            host_list = host_list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py(px(16.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("No matching hosts"),
            );
        }

        // Add connection button (pinned at bottom)
        let add_button = div()
            .id("add-connection-btn")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .w_full()
            .overflow_hidden()
            .px(px(12.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .on_click(cx.listener(|_this, _event: &ClickEvent, _window, cx| {
                cx.emit(SidebarEvent::AddConnection);
            }))
            .child(Button::new("add-connection", "+ Add Connection").variant(ButtonVariant::Ghost));

        // Invisible resize hit-area overlapping the right border.
        let resize_handle = div()
            .id("sidebar-resize-handle")
            .absolute()
            .right(px(-3.0))
            .top_0()
            .w(px(6.0))
            .h_full()
            .cursor_col_resize()
            .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.4)))
            .when(self.resizing, |el| {
                el.bg(ShellDeckColors::primary().opacity(0.6))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    this.resizing = true;
                    cx.notify();
                }),
            );

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.width))
            .h_full()
            .overflow_hidden()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .id("sidebar")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _window, cx| {
                if search_focused {
                    this.handle_search_key_down(event, cx);
                }
            }))
            .child(logo)
            .child(nav)
            .child(host_list)
            .child(add_button)
            .child(resize_handle)
    }
}
