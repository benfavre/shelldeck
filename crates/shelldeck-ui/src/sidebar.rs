use crate::scale::px;
use gpui::prelude::*;
use gpui::*;

use adabraka_ui::components::input::{Input, InputSize, InputState};

use adabraka_ui::prelude::*;
use shelldeck_core::models::connection::{Connection, ConnectionStatus};
use std::collections::HashMap;
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

/// Width of the VS Code style activity rail, in **real** pixels.
///
/// Deliberately absolute, like the panel width right below it: the terminal
/// grid is sized against the sidebar's total width, so a rail that grew with
/// the UI scale would need that scale threaded through every consumer. The
/// icons inside it are pinned to real pixels for the same reason — a rem-sized
/// glyph would outgrow a fixed 48px rail at 2× and collide with its edges
/// (`.agents/spacing.md`).
pub const RAIL_WIDTH: f32 = 48.0;

/// Pure helper: total horizontal space the sidebar occupies.
///
/// Extracted from [`SidebarView::total_width`] so the arithmetic the terminal
/// grid depends on can be tested without a GPUI `Context` — same reasoning as
/// `conn_matches_site_filter` above.
///
/// - `nav_collapsed`: the activity rail is hidden.
/// - `panel_collapsed`: the panel is hidden (Cmd/Ctrl+B).
/// - `section_has_panel`: the selected activity has contextual rows at all.
///   An activity without one hides the panel even when it is not collapsed,
///   so the terminal must be offset by the rail alone.
fn sidebar_total_width(
    nav_collapsed: bool,
    panel_collapsed: bool,
    section_has_panel: bool,
    panel_width: f32,
) -> f32 {
    let rail = if nav_collapsed { 0.0 } else { RAIL_WIDTH };
    let panel = if panel_collapsed || !section_has_panel {
        0.0
    } else {
        panel_width
    };
    rail + panel
}

/// One row in the contextual panel below a rail activity.
///
/// Every non-Connections activity feeds the panel through this shape rather
/// than growing its own renderer: the panel is a *list of things you can jump
/// to*, and that is the same widget whatever the activity. Connections keeps
/// its bespoke renderer (groups, pins, per-row hover actions, site badges).
#[derive(Debug, Clone, PartialEq)]
pub struct PanelItem {
    pub id: Uuid,
    pub label: String,
    /// Secondary line — host, path, port pair, timestamp. Optional.
    pub detail: Option<String>,
    /// Lucide slug from the bundled subset (`.agents/icons.md`).
    pub icon: &'static str,
    /// Currently selected / open — renders like the active rail item.
    pub is_active: bool,
    /// Live: connected session, running forward, unsaved buffer. Renders a
    /// success-tinted dot.
    pub is_live: bool,
}

/// Navigation sections in the sidebar
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// The activities that earn a slot in the rail, in order.
    ///
    /// Deliberately *not* every section: an activity bar lists places with a
    /// contextual panel behind them. JeanClaude, Fleet and bext Cloud are
    /// destinations, not activities — they are reached from the Aller menu and
    /// the command palette. `Settings` is excluded here because the rail pins
    /// it separately at the bottom.
    pub fn rail_activities() -> &'static [SidebarSection] {
        &[
            SidebarSection::Connections,
            SidebarSection::Terminals,
            SidebarSection::Scripts,
            SidebarSection::PortForwards,
            SidebarSection::ServerSync,
            SidebarSection::Sites,
            SidebarSection::Recent,
            SidebarSection::FileEditor,
        ]
    }

    /// Whether this activity backs its rail icon with a contextual panel.
    ///
    /// `false` means selecting it switches the main view and collapses the
    /// panel, so the view gets the full width instead of sitting next to a
    /// list that has nothing to say.
    pub fn has_panel(&self) -> bool {
        matches!(
            self,
            SidebarSection::Connections
                | SidebarSection::Terminals
                | SidebarSection::Scripts
                | SidebarSection::PortForwards
                | SidebarSection::Sites
                | SidebarSection::Recent
                | SidebarSection::FileEditor
        )
    }

    /// Localized empty-state line for an activity whose panel has no rows yet.
    pub fn empty_hint(&self) -> String {
        match self {
            SidebarSection::Terminals => t!("sidebar.empty.terminals"),
            SidebarSection::Scripts => t!("sidebar.empty.scripts"),
            SidebarSection::PortForwards => t!("sidebar.empty.port_forwards"),
            SidebarSection::Sites => t!("sidebar.empty.sites"),
            SidebarSection::Recent => t!("sidebar.empty.recent"),
            SidebarSection::FileEditor => t!("sidebar.empty.editor"),
            _ => t!("sidebar.empty.generic"),
        }
        .to_string()
    }

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
    /// A row in a contextual panel was clicked. The workspace decides what
    /// "select" means per activity (focus a terminal tab, open a script, jump
    /// to a forward, …) — the sidebar only reports which row in which
    /// activity.
    PanelItemSelected {
        section: SidebarSection,
        id: Uuid,
    },
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
    /// Contextual panel rows per activity, pushed by the workspace. Keyed by
    /// section so the panel can render whichever activity is selected without
    /// the workspace having to re-push on every switch.
    panel_items: HashMap<SidebarSection, Vec<PanelItem>>,
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
            panel_items: HashMap::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// Replace the contextual panel rows for one activity.
    pub fn set_panel_items(&mut self, section: SidebarSection, items: Vec<PanelItem>) {
        self.panel_items.insert(section, items);
    }

    /// Rows currently held for an activity.
    pub fn panel_items(&self, section: SidebarSection) -> &[PanelItem] {
        self.panel_items
            .get(&section)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
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

    /// Total horizontal space the sidebar occupies: the activity rail (unless
    /// hidden) plus the panel (unless collapsed).
    ///
    /// This — not [`Self::width`], which is the panel alone — is what the
    /// terminal grid must be offset by. Kept as one function so the rail and
    /// panel can never disagree about the total.
    pub fn total_width(&self) -> f32 {
        sidebar_total_width(
            self.nav_collapsed,
            self.collapsed,
            self.active_section.has_panel(),
            self.width,
        )
    }

    /// Width of the rail alone (0 when hidden). Used by the resize drag to
    /// convert a window-space mouse X into a panel width.
    pub fn rail_offset(&self) -> f32 {
        if self.nav_collapsed {
            0.0
        } else {
            RAIL_WIDTH
        }
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

    /// One icon button in the activity rail. Shows the section label as a
    /// hover tooltip, since the rail has no room for text.
    fn render_rail_item(
        &self,
        section: SidebarSection,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_section == section;
        let icon = section.lucide_icon();
        let label: SharedString = section.label().into();
        let icon_color = if is_active {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::text_muted()
        };

        // Real pixels throughout: see RAIL_WIDTH. The 32px hit target inside a
        // 48px rail leaves 8px of breathing room on each side.
        let mut item = div()
            .id(ElementId::from(SharedString::from(format!(
                "rail-{section:?}"
            ))))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .size(gpui::px(32.0))
            .rounded(gpui::px(8.0))
            .cursor_pointer()
            .tooltip(move |_, cx| {
                cx.new(|_| SidebarTooltip {
                    label: label.clone(),
                })
                .into()
            })
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                this.active_section = section;
                cx.emit(SidebarEvent::SectionChanged(section));
                cx.notify();
            }))
            .child(lucide_icon(icon, 18.0, icon_color));

        if is_active {
            item = item.bg(ShellDeckColors::primary().opacity(0.15));
            // Active marker on the left edge, VS Code style. Inset vertically
            // so it reads as a marker rather than a full-height divider.
            item = item.child(
                div()
                    .absolute()
                    .left(gpui::px(-8.0))
                    .top(gpui::px(6.0))
                    .bottom(gpui::px(6.0))
                    .w(gpui::px(2.0))
                    .rounded(gpui::px(1.0))
                    .bg(ShellDeckColors::primary()),
            );
        } else {
            item = item.hover(|el| el.bg(ShellDeckColors::hover_bg()));
        }

        // Count badge, VS Code style. Carries the connected-host and open-tab
        // counts the in-panel nav rows used to show, so collapsing to the rail
        // is not an information loss.
        if let Some(count) = count.filter(|c| *c > 0) {
            let label = if count > 99 {
                "99+".to_string()
            } else {
                count.to_string()
            };
            item = item.child(
                div()
                    .absolute()
                    .top(gpui::px(-2.0))
                    .right(gpui::px(-2.0))
                    .min_w(gpui::px(15.0))
                    .h(gpui::px(15.0))
                    .px(gpui::px(3.0))
                    .rounded(gpui::px(7.5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(ShellDeckColors::primary())
                    .text_size(gpui::px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::bg_primary())
                    .child(label),
            );
        }
        item
    }

    /// The always-on activity rail: every nav section as an icon, with
    /// Settings pinned to the bottom.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connected_count = self
            .connections
            .iter()
            .filter(|c| matches!(c.status, ConnectionStatus::Connected))
            .count();

        let mut top = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(gpui::px(4.0));
        for &section in SidebarSection::rail_activities() {
            // Badges come from the activity's own data, so a section with
            // nothing to count simply has none.
            let count = match section {
                SidebarSection::Connections => Some(connected_count),
                SidebarSection::Terminals => Some(self.terminal_tab_count),
                _ => None,
            };
            top = top.child(self.render_rail_item(section, count, cx));
        }

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .w(gpui::px(RAIL_WIDTH))
            .h_full()
            .py(gpui::px(8.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(gpui::px(8.0))
                    .min_h(gpui::px(0.0))
                    .overflow_hidden()
                    .child(crate::brand::brand_badge_abs(24.0))
                    .child(top),
            )
            .child(self.render_rail_item(SidebarSection::Settings, None, cx))
    }

    /// One row of a contextual panel. Deliberately shaped like a connection
    /// row (same paddings, same icon-then-label-then-status order) so the
    /// panel does not visually re-invent itself per activity —
    /// `.agents/ui-components.md` § Harmonization.
    fn render_panel_item(
        &self,
        section: SidebarSection,
        item: &PanelItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = item.id;
        let icon_color = if item.is_active {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::text_muted()
        };

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "panel-{section:?}-{id}"
            ))))
            .flex()
            .items_center()
            .gap(px(8.0))
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .on_click(cx.listener(move |_this, _event: &ClickEvent, _window, cx| {
                cx.emit(SidebarEvent::PanelItemSelected { section, id });
                cx.notify();
            }))
            .child(lucide_icon(item.icon, 14.0, icon_color));

        if item.is_active {
            row = row
                .bg(ShellDeckColors::primary().opacity(0.15))
                .text_color(ShellDeckColors::primary());
        } else {
            row = row
                .text_color(ShellDeckColors::text_primary())
                .hover(|el| el.bg(ShellDeckColors::hover_bg()));
        }

        // Label + optional detail share one shrinking column; only this column
        // shrinks, so long paths never push the status dot off the row
        // (`.agents/overflow.md`).
        let mut text_col = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(
                div()
                    .text_size(px(12.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(item.label.clone()),
            );
        if let Some(detail) = &item.detail {
            text_col = text_col.child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(detail.clone()),
            );
        }
        row = row.child(text_col);

        if item.is_live {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(ShellDeckColors::success()),
            );
        }
        row
    }

    /// The contextual panel body for the selected activity: the bespoke
    /// connection list for Connections, a generic `PanelItem` list otherwise.
    fn render_panel_body(&self, cx: &mut Context<Self>) -> Div {
        let section = self.active_section;
        let items = self.panel_items(section);

        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .px(px(4.0))
            .py(px(4.0));

        if items.is_empty() {
            return list.child(
                div()
                    .px(px(10.0))
                    .py(px(12.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(section.empty_hint()),
            );
        }
        for item in items {
            list = list.child(self.render_panel_item(section, item, cx));
        }
        list
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
        // VS Code layout: an always-on activity rail plus a panel that
        // collapses independently (Cmd/Ctrl+B). `nav_collapsed` — the setting
        // that used to hide the in-panel nav list, which the rail replaced —
        // now hides the rail, so "masquer la navigation" still means the same
        // thing to anyone who had it turned on.
        let rail_visible = !self.nav_collapsed;
        // An activity with no contextual list (Server Sync, and the
        // destinations reachable only from the menu) hides the panel entirely
        // so its main view gets the full width, rather than parking an empty
        // column next to it.
        let panel_visible = !self.collapsed && self.active_section.has_panel();
        let on_connections = self.active_section == SidebarSection::Connections;

        if !panel_visible {
            let mut rail_only = div()
                .flex()
                .flex_shrink_0()
                .h_full()
                .id("sidebar-rail-only");
            if rail_visible {
                rail_only = rail_only.child(self.render_rail(cx));
            }
            return rail_only;
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

        // Panel header. With the rail on screen the brand mark already lives
        // there, so the header names the section the panel is showing (VS Code
        // style). With the rail hidden it falls back to the full brand lockup,
        // because that is then the only place it appears.
        //
        // The rail toggle lives here, right-aligned, because it is *sidebar*
        // chrome. It previously sat as a full-width bordered strip immediately
        // above the hosts list, which read as that list's own collapse control
        // — and once the panel became contextual it appeared above every
        // activity, offering to "hide the navigation" from the middle of a
        // list of scripts.
        let mut header_left = div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .items_center()
            .gap(px(10.0))
            .overflow_hidden();
        if rail_visible {
            header_left = header_left.child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(self.active_section.label().to_uppercase()),
            );
        } else {
            header_left = header_left
                .child(crate::brand::brand_badge(24.0))
                .child(crate::brand::brand_wordmark(15.0));
        }

        let toggle_hint: SharedString = if rail_visible {
            t!("sidebar.hide_nav").to_string().into()
        } else {
            t!("sidebar.show_nav").to_string().into()
        };
        let rail_toggle = div()
            .id("sidebar-rail-toggle")
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .size(px(22.0))
            .rounded(px(5.0))
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .tooltip(move |_, cx| {
                cx.new(|_| SidebarTooltip {
                    label: toggle_hint.clone(),
                })
                .into()
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.nav_collapsed = !this.nav_collapsed;
                cx.emit(SidebarEvent::NavCollapsedChanged(this.nav_collapsed));
                cx.notify();
            }))
            .child(lucide_icon(
                if rail_visible {
                    "chevron-left"
                } else {
                    "chevron-right"
                },
                12.0,
                ShellDeckColors::text_muted(),
            ));

        let logo = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.0))
            .w_full()
            .overflow_hidden()
            .px(px(12.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(header_left)
            .child(rail_toggle);

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

        // Scrollable host list (fills remaining space, wrapped in scrollable_vertical below).
        // No "HÔTES" header: the panel header already names the active
        // activity, and stacking CONNEXIONS above HÔTES labelled the same list
        // twice.
        let mut host_list = div()
            .flex()
            .flex_col()
            .id("sidebar-host-list")
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
        // The rail *is* the navigation. Only fall back to the in-panel nav
        // list when the user has hidden the rail, so the two never duplicate
        // each other.
        if !rail_visible {
            root = root.child(nav);
        }
        // The panel body follows the selected activity. Connections keeps its
        // bespoke list (groups, pins, per-row actions, site badges); every
        // other activity renders its `PanelItem` rows. Without this the panel
        // showed the host list under whatever header was selected — a list
        // that flatly contradicted its own title.
        let body = if on_connections {
            scrollable_vertical(host_list).into_any_element()
        } else {
            scrollable_vertical(self.render_panel_body(cx)).into_any_element()
        };

        let mut panel = root.child(
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
                .child(body),
        );
        // "+ Ajouter une connexion" belongs to the Connections activity only.
        if on_connections {
            panel = panel.child(add_button);
        }
        let panel = panel.child(resize_handle);

        let mut shell = div().flex().flex_shrink_0().h_full().id("sidebar-shell");
        if rail_visible {
            shell = shell.child(self.render_rail(cx));
        }
        shell.child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::{conn_matches_site_filter, fuzzy_match_indices, sidebar_total_width, RAIL_WIDTH};
    use uuid::Uuid;

    // ── sidebar_total_width ────────────────────────────────────────────

    // SDTEST-1210 — the terminal grid is sized against this number. Each of
    // the four rail/panel states must contribute exactly its own width; an
    // error here silently mis-sizes every terminal (wrong cols, wrong
    // wrapping) rather than showing up as a visible layout break.
    #[test]
    fn total_width_sums_rail_and_panel_independently() {
        // Both visible — the default Dev layout.
        assert_eq!(
            sidebar_total_width(false, false, true, 260.0),
            RAIL_WIDTH + 260.0
        );
        // Panel collapsed (Cmd+B): the rail stays, which is the whole point
        // of the VS Code layout — a plain 0.0 here would put the terminal
        // underneath the rail.
        assert_eq!(sidebar_total_width(false, true, true, 260.0), RAIL_WIDTH);
        // Rail hidden ("masquer la navigation"), panel still open.
        assert_eq!(sidebar_total_width(true, false, true, 260.0), 260.0);
        // Everything hidden.
        assert_eq!(sidebar_total_width(true, true, true, 260.0), 0.0);
    }

    // SDTEST-1211 — a collapsed panel must not leak its width back in, at
    // either end of the resize clamp.
    #[test]
    fn collapsed_panel_width_is_ignored_at_any_size() {
        for width in [180.0, 260.0, 400.0] {
            assert_eq!(sidebar_total_width(false, true, true, width), RAIL_WIDTH);
            assert_eq!(sidebar_total_width(true, true, true, width), 0.0);
        }
    }

    // SDTEST-1213 — an activity with no contextual list hides the panel even
    // when the panel is not collapsed. If the width still counted the panel,
    // the terminal would be offset past a column that is not on screen and
    // every grid would be sized short by the panel width.
    #[test]
    fn activity_without_a_panel_contributes_no_panel_width() {
        assert_eq!(sidebar_total_width(false, false, false, 260.0), RAIL_WIDTH);
        assert_eq!(sidebar_total_width(true, false, false, 260.0), 0.0);
        // Still zero when also collapsed — the two reasons must not add up.
        assert_eq!(sidebar_total_width(false, true, false, 260.0), RAIL_WIDTH);
    }

    // SDTEST-1214 — the rail lists activities that have a panel behind them,
    // plus Server Sync as a deliberate main-view-only entry. The three
    // destinations reached from the Aller menu must never take a rail slot,
    // and Settings is pinned separately rather than living in the list.
    #[test]
    fn rail_lists_activities_not_destinations() {
        let rail = super::SidebarSection::rail_activities();
        for excluded in [
            super::SidebarSection::JeanConsole,
            super::SidebarSection::Fleet,
            super::SidebarSection::BextCloud,
            super::SidebarSection::Settings,
        ] {
            assert!(
                !rail.contains(&excluded),
                "{excluded:?} is a destination, not a rail activity"
            );
        }
        // Every rail entry either has a panel or is the known exception.
        for section in rail {
            assert!(
                section.has_panel() || *section == super::SidebarSection::ServerSync,
                "{section:?} sits in the rail with neither a panel nor an exemption"
            );
        }
    }

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
