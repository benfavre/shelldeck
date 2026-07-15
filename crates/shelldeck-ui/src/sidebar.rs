use crate::scale::px;
use gpui::prelude::*;
use gpui::*;

use adabraka_ui::components::input::{Input, InputSize, InputState};

use adabraka_ui::prelude::*;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use uuid::Uuid;

use crate::command_palette::fuzzy_match;
use crate::icons::lucide_icon;
use crate::t;
use crate::theme::ShellDeckColors;

struct SidebarTooltip {
    label: SharedString,
}

impl Render for SidebarTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let font_family = use_theme().tokens.font_family.clone();
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .shadow_md()
            .text_size(px(11.0))
            .font_family(font_family)
            .text_color(ShellDeckColors::text_primary())
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}

/// Pure helper: whether a connection passes the sidebar's active-site
/// filter. Extracted from `SidebarView::conn_matches_site` so unit tests
/// don't need a GPUI `Context` to exercise the contract.
///
/// - `None` filter: everything passes.
/// - `Some(active)` filter: the connection passes iff it is bound to
///   `active` OR it is unbound (`conn_site_id.is_none()`) — the "manual /
///   ssh-config / cloud-without-site connections always show" rule from
///   AGENTS.md § 7.
fn conn_matches_site_filter(site_filter: Option<Uuid>, conn_site_id: Option<Uuid>) -> bool {
    match site_filter {
        None => true,
        Some(active) => conn_site_id == Some(active) || conn_site_id.is_none(),
    }
}

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
    Terminals,
    Scripts,
    PortForwards,
    ServerSync,
    Sites,
    Recent,
    FileEditor,
    JeanConsole,
    Fleet,
    BextCloud,
    Settings,
}

impl SidebarSection {
    /// Lucide slug for the Dev sidebar nav row (see `icons/lucide/` inventory).
    pub fn lucide_icon(&self) -> &'static str {
        match self {
            SidebarSection::Connections => "server",
            SidebarSection::Terminals => "terminal",
            SidebarSection::Scripts => "scroll-text",
            SidebarSection::PortForwards => "arrow-left-right",
            SidebarSection::ServerSync => "refresh-cw",
            SidebarSection::Sites => "globe",
            SidebarSection::Recent => "activity",
            SidebarSection::FileEditor => "pencil",
            SidebarSection::JeanConsole => "cpu",
            SidebarSection::Fleet => "box",
            SidebarSection::BextCloud => "cloud",
            SidebarSection::Settings => "settings",
        }
    }

    pub fn label(&self) -> String {
        match self {
            SidebarSection::Connections => t!("sidebar.nav.connections"),
            SidebarSection::Terminals => t!("sidebar.nav.terminals"),
            SidebarSection::Scripts => t!("sidebar.nav.scripts"),
            SidebarSection::PortForwards => t!("sidebar.nav.port_forwards"),
            SidebarSection::ServerSync => t!("sidebar.nav.server_sync"),
            SidebarSection::Sites => t!("sidebar.nav.sites"),
            SidebarSection::Recent => t!("sidebar.nav.recent"),
            SidebarSection::FileEditor => t!("sidebar.nav.editor"),
            SidebarSection::JeanConsole => t!("sidebar.nav.jean"),
            SidebarSection::Fleet => t!("sidebar.nav.fleet"),
            SidebarSection::BextCloud => t!("sidebar.nav.bext"),
            SidebarSection::Settings => t!("sidebar.nav.settings"),
        }
        .to_string()
    }
}

/// Events emitted by the sidebar
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    ConnectionSelected(Uuid),
    ConnectionConnect(Uuid),
    ConnectionEdit(Uuid),
    ConnectionDelete(Uuid),
    ConnectionPinToggled(Uuid),
    /// Manage the bext instance behind this connection (loopback site SDK).
    ConnectionManageBext(Uuid),
    /// Open the row's kebab (⋮) action menu at the given window position.
    OpenConnectionMenu {
        conn_id: Uuid,
        position: Point<Pixels>,
    },
    AddConnection,
    SectionChanged(SidebarSection),
    QuickConnect,
    WidthChanged(f32),
    /// User toggled the top-nav collapse chevron — workspace persists this
    /// to `AppConfig.general.sidebar_nav_collapsed` so the layout sticks
    /// across sessions.
    NavCollapsedChanged(bool),
}

pub struct SidebarView {
    connections: Vec<Connection>,
    pinned_connections: Vec<Uuid>,
    selected_connection: Option<Uuid>,
    active_section: SidebarSection,
    collapsed: bool,
    /// Whether the top navigation section is collapsed. When true, only the
    /// hosts section (search + list) remains visible. Persisted by the
    /// workspace via `AppConfig.general.sidebar_nav_collapsed`.
    nav_collapsed: bool,
    width: f32,
    /// Whether the user is currently dragging the resize handle.
    resizing: bool,
    /// Number of open terminal tabs (shown as badge)
    terminal_tab_count: usize,
    /// Host search query
    search_state: Entity<InputState>,
    /// Cached snapshot of the current input value (used by `conn_matches_search`
    /// and highlight helpers). Kept in sync with `search_state` via the Input
    /// widget's `on_change` callback.
    search_query: String,
    /// Active Inklura Manage site filter. `Some(id)` hides connections bound to
    /// a *different* site (unbound connections always show); `None` = all sites.
    site_filter: Option<Uuid>,
    /// Whether the JeanClaude console nav entry should be shown (config present).
    jean_available: bool,
    /// Whether the Jean fleet nav entry should be shown (Dev + signed in).
    fleet_available: bool,
    focus_handle: FocusHandle,
}

impl SidebarView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            connections: Vec::new(),
            pinned_connections: Vec::new(),
            selected_connection: None,
            active_section: SidebarSection::Connections,
            collapsed: false,
            nav_collapsed: false,
            width: 260.0,
            resizing: false,
            terminal_tab_count: 0,
            search_state: cx.new(InputState::new),
            search_query: String::new(),
            site_filter: None,
            jean_available: false,
            fleet_available: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Seed the persisted "top nav collapsed" state from the app config.
    /// Called by the workspace on init.
    pub fn set_nav_collapsed(&mut self, collapsed: bool) {
        self.nav_collapsed = collapsed;
    }

    /// Show/hide the JeanClaude console nav entry (Dev mode + config present).
    pub fn set_jean_available(&mut self, available: bool) {
        self.jean_available = available;
    }

    /// Show/hide the Jean fleet nav entry (Dev mode + signed in).
    pub fn set_fleet_available(&mut self, available: bool) {
        self.fleet_available = available;
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

    pub fn set_pinned_connections(&mut self, pinned_connections: Vec<Uuid>) {
        self.pinned_connections = pinned_connections;
    }

    /// Highlight a connection in the Connections section without opening an
    /// SSH session. Used by the `shelldeck://open/connection/<uuid>` deep
    /// link so a link can point the user at a connection without connecting.
    pub fn focus_connection(&mut self, id: Uuid) {
        self.active_section = SidebarSection::Connections;
        self.selected_connection = Some(id);
    }

    /// Set the active-site filter. `Some(id)` scopes the list to that site
    /// (plus unbound connections); `None` shows every site.
    pub fn set_site_filter(&mut self, site_filter: Option<Uuid>) {
        self.site_filter = site_filter;
    }

    /// Whether `conn` passes the active-site filter: no filter, an exact site
    /// match, or an unbound connection (manual / ssh / cloud-without-site).
    fn conn_matches_site(&self, conn: &Connection) -> bool {
        conn_matches_site_filter(self.site_filter, conn.site_id)
    }

    pub fn set_terminal_tab_count(&mut self, count: usize) {
        self.terminal_tab_count = count;
    }

    pub fn set_active_section(&mut self, section: SidebarSection) {
        self.active_section = section;
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
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = section.label();
        let is_active = self.active_section == section;
        let icon = section.lucide_icon();
        let icon_color = if is_active {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::text_muted()
        };

        div()
            .id(ElementId::from(SharedString::from(format!(
                "nav-{section:?}"
            ))))
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .overflow_hidden()
            .px(px(10.0))
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
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(lucide_icon(icon, 14.0, icon_color))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(label.to_string()),
                    ),
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
        // Real adabraka `Input` — cursor, selection, Ctrl+C/V/X, built-in
        // clear button. The magnifying-glass prefix keeps the affordance we
        // had with the fake-input version.
        let input = Input::new(&self.search_state)
            .size(InputSize::Sm)
            .placeholder(t!("sidebar.filter_placeholder").to_string())
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
            });

        div().flex_shrink_0().px(px(8.0)).py(px(6.0)).child(input)
    }

    fn render_connection_item_highlighted(
        &self,
        connection: &Connection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_connection == Some(connection.id);
        let conn_id = connection.id;
        let is_pinned = self.pinned_connections.contains(&conn_id);
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

        // Kebab button — faint hint always visible so the affordance is
        // discoverable, brightens on row hover. Click opens a dropdown at the
        // click position with SSH/Edit/bext/Delete.
        let pin_tooltip = if is_pinned {
            t!("sidebar.unpin_connection").to_string()
        } else {
            t!("sidebar.pin_connection").to_string()
        };
        let pin_button = div()
            .id(ElementId::from(SharedString::from(format!(
                "conn-pin-{conn_id}"
            ))))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.0))
            .h(px(22.0))
            .rounded(px(4.0))
            .text_color(if is_pinned {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::text_muted()
            })
            .opacity(if is_pinned { 1.0 } else { 0.0 })
            .group_hover(group_name.clone(), |el| el.opacity(1.0))
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .tooltip(move |_, cx| {
                cx.new(|_| SidebarTooltip {
                    label: pin_tooltip.clone().into(),
                })
                .into()
            })
            .on_click(cx.listener(move |_this, _event: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.emit(SidebarEvent::ConnectionPinToggled(conn_id));
            }))
            .child(
                svg()
                    .path("icons/lucide/pin.svg")
                    .size(px(13.0))
                    .text_color(if is_pinned {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::text_muted()
                    }),
            );

        let action_buttons = div()
            .id(ElementId::from(SharedString::from(format!(
                "conn-kebab-{}",
                conn_id
            ))))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.0))
            .h(px(22.0))
            .mr(px(4.0))
            .rounded(px(4.0))
            .text_size(px(16.0))
            .font_weight(FontWeight::BOLD)
            .text_color(ShellDeckColors::text_muted())
            .opacity(0.35)
            .group_hover(group_name.clone(), |el| el.opacity(1.0))
            .cursor_pointer()
            .hover(|el| {
                el.bg(ShellDeckColors::hover_bg())
                    .text_color(ShellDeckColors::text_primary())
            })
            .on_click(cx.listener(move |_this, event: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.emit(SidebarEvent::OpenConnectionMenu {
                    conn_id,
                    position: event.position(),
                });
            }))
            .child(
                svg()
                    .path("icons/lucide/ellipsis-vertical.svg")
                    .size(px(14.0))
                    .text_color(ShellDeckColors::text_muted()),
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

        // Name line: the name plus an optional Manage-site badge.
        let mut name_row = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .min_w(px(0.0))
            .overflow_hidden()
            .child(name_el);
        if let Some(label) = connection
            .site_label
            .as_ref()
            .filter(|l| !l.trim().is_empty())
        {
            name_row = name_row.child(
                div()
                    .flex_shrink_0()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::badge_bg())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .max_w(px(88.0))
                    .child(label.clone()),
            );
        }

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
                    .child(name_row)
                    .child(conn_str_el),
            );

        row = row.child(content);
        row = row.child(pin_button);
        row = row.child(action_buttons);
        row
    }
}

impl EventEmitter<SidebarEvent> for SidebarView {}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.collapsed {
            return div().w(px(0.0)).h_full().id("sidebar-collapsed");
        }

        // Filter connections by search query and the active-site filter.
        let filtered: Vec<&Connection> = self
            .connections
            .iter()
            .filter(|c| self.conn_matches_site(c) && self.conn_matches_search(c))
            .collect();

        // Group filtered connections by group
        let mut grouped: std::collections::BTreeMap<String, Vec<&Connection>> =
            std::collections::BTreeMap::new();
        let mut ungrouped: Vec<&Connection> = Vec::new();

        for conn in &filtered {
            if self.pinned_connections.contains(&conn.id) {
                continue;
            }
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
            .child(crate::brand::brand_badge(30.0))
            .child(crate::brand::brand_wordmark(17.0));

        // Navigation tabs (pinned at top)
        let mut nav = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(8.0))
            .child(self.render_nav_item(SidebarSection::Connections, Some(connected_count), cx))
            .child(self.render_nav_item(
                SidebarSection::Terminals,
                if self.terminal_tab_count > 0 {
                    Some(self.terminal_tab_count)
                } else {
                    None
                },
                cx,
            ))
            .child(self.render_nav_item(SidebarSection::Scripts, None, cx))
            .child(self.render_nav_item(SidebarSection::PortForwards, None, cx))
            .child(self.render_nav_item(SidebarSection::ServerSync, None, cx))
            .child(self.render_nav_item(SidebarSection::Sites, None, cx))
            .child(self.render_nav_item(SidebarSection::Recent, None, cx))
            .child(self.render_nav_item(SidebarSection::FileEditor, None, cx));
        if self.jean_available {
            nav = nav.child(self.render_nav_item(SidebarSection::JeanConsole, None, cx));
        }
        if self.fleet_available {
            nav = nav.child(self.render_nav_item(SidebarSection::Fleet, None, cx));
        }
        nav = nav.child(self.render_nav_item(SidebarSection::BextCloud, None, cx));
        nav = nav.child(self.render_nav_item(SidebarSection::Settings, None, cx));

        // Scrollable host list (fills remaining space, wrapped in scrollable_vertical below)
        let mut host_list = div()
            .flex()
            .flex_col()
            .id("sidebar-host-list")
            .child(Self::render_section_header(t!("sidebar.hosts").as_ref()))
            .child(self.render_search_bar(cx));

        let pinned: Vec<&Connection> = self
            .pinned_connections
            .iter()
            .filter_map(|id| filtered.iter().copied().find(|conn| conn.id == *id))
            .collect();
        if !pinned.is_empty() {
            host_list = host_list.child(Self::render_section_header(t!("sidebar.pinned").as_ref()));
            for conn in pinned {
                host_list = host_list.child(self.render_connection_item_highlighted(conn, cx));
            }
            if !ungrouped.is_empty() || !grouped.is_empty() {
                host_list = host_list.child(Self::render_section_header(
                    t!("sidebar.other_hosts").as_ref(),
                ));
            }
        }

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
                    .child(t!("sidebar.no_matches").to_string()),
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
            .child(
                Button::new("add-connection", t!("sidebar.add_connection").to_string())
                    .variant(ButtonVariant::Ghost),
            );

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

        // Collapsible top-nav separator: a click-to-toggle chevron sitting
        // between the nav section and the hosts list. Persisted via
        // `SidebarEvent::NavCollapsedChanged`.
        let nav_collapsed = self.nav_collapsed;
        let separator = div()
            .id("sidebar-nav-toggle")
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(4.0))
            .cursor_pointer()
            .border_t_1()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.nav_collapsed = !this.nav_collapsed;
                cx.emit(SidebarEvent::NavCollapsedChanged(this.nav_collapsed));
                cx.notify();
            }))
            .child(
                svg()
                    .path("icons/lucide/chevron-down.svg")
                    .size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .with_transformation(gpui::Transformation::rotate(gpui::radians(
                        if nav_collapsed {
                            std::f32::consts::PI
                        } else {
                            0.0
                        },
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(if nav_collapsed {
                        t!("sidebar.show_nav").to_string()
                    } else {
                        t!("sidebar.hide_nav").to_string()
                    }),
            );

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            // Real pixels: the terminal's grid_x_offset depends on this exact
            // width, so it must not rem-scale with the rest of the sidebar.
            .w(gpui::px(self.width))
            .h_full()
            .overflow_hidden()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .id("sidebar")
            .track_focus(&self.focus_handle)
            .child(logo);
        if !nav_collapsed {
            root = root.child(nav);
        }
        root.child(separator)
            .child(
                // Explicit flex-grow + min_h(0) around the scrollable so the
                // scroll container computes its viewport height correctly and
                // stops clipping the last row above the "+ Add Connection"
                // footer.
                div()
                    .flex()
                    .flex_col()
                    .flex_grow()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(scrollable_vertical(host_list)),
            )
            .child(add_button)
            .child(resize_handle)
    }
}

#[cfg(test)]
mod tests {
    use super::{conn_matches_site_filter, fuzzy_match_indices};
    use uuid::Uuid;

    // ── fuzzy_match_indices ────────────────────────────────────────────

    // SDTEST-1020 — empty needle: Some(vec![]) means "matches, no highlights".
    // Distinct from None (no match).
    #[test]
    fn empty_needle_returns_empty_indices() {
        assert_eq!(fuzzy_match_indices("anything", ""), Some(vec![]));
        assert_eq!(fuzzy_match_indices("", ""), Some(vec![]));
    }

    // SDTEST-1021 — returned indices are CHAR positions in the lowercased
    // haystack (not byte positions). The highlighter walks a `Vec<char>` at
    // the same char index, so this is the contract the renderer relies on.
    // A byte-index return would misalign accented labels ("Créer" — 'é' is 2
    // bytes, so byte-index 2 = middle of the accent, not the third char).
    #[test]
    fn returns_char_positions_not_bytes() {
        assert_eq!(fuzzy_match_indices("abcdef", "ace"), Some(vec![0, 2, 4]));
        // 'é' is 2 bytes but 1 char: pos 3 (char) vs 4 (byte).
        // Needle "cé" matches at chars [0, 2] in "créer".
        assert_eq!(fuzzy_match_indices("créer", "cé"), Some(vec![0, 2]));
    }

    // SDTEST-1022 — no match returns None (distinct from empty Some).
    #[test]
    fn no_match_returns_none() {
        assert_eq!(fuzzy_match_indices("abc", "d"), None);
        assert_eq!(fuzzy_match_indices("abc", "abcd"), None);
        // Case sensitivity: both sides lowercased, so uppercase in needle
        // is fine (unlike command_palette::fuzzy_match).
        assert_eq!(fuzzy_match_indices("abc", "ABC"), Some(vec![0, 1, 2]));
    }

    // ── conn_matches_site_filter ───────────────────────────────────────

    // SDTEST-1023 — no active site filter shows everything.
    #[test]
    fn no_filter_matches_every_connection() {
        let bound = Some(Uuid::new_v4());
        assert!(conn_matches_site_filter(None, bound));
        assert!(conn_matches_site_filter(None, None));
    }

    // SDTEST-1024 — filter set: matches the exact site AND every unbound
    // connection (manual / ssh-config / cloud-without-site). Contract per
    // AGENTS.md § 7.
    #[test]
    fn filter_matches_bound_site_and_all_unbound_connections() {
        let active = Uuid::new_v4();
        let other = Uuid::new_v4();
        // exact site match
        assert!(conn_matches_site_filter(Some(active), Some(active)));
        // unbound connection (no site_id) is always visible
        assert!(conn_matches_site_filter(Some(active), None));
        // different site is filtered out
        assert!(!conn_matches_site_filter(Some(active), Some(other)));
    }
}
