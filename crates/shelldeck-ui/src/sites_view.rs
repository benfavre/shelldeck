use std::collections::HashSet;
use std::ops::Range;

use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::Connection;
use shelldeck_core::models::managed_site::{ManagedSite, ManagedSiteType, SiteStatus};
use shelldeck_core::models::server_sync::DatabaseEngine;
use uuid::Uuid;

const PAGE_SIZE: usize = 50;

use crate::theme::ShellDeckColors;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events emitted by SitesView to be handled by Workspace.
#[derive(Debug, Clone)]
pub enum SitesEvent {
    ScanServer(Uuid),
    ScanAllServers,
    CheckSiteStatus(Uuid),
    RemoveSite(Uuid),
    ToggleFavorite(Uuid),
    UpdateTags(Uuid, Vec<String>),
    OpenInBrowser(String),
    SshToServer(Uuid),
    AddToSync(Uuid),
    RefreshSites,
    ClearAllSites,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SitesViewMode {
    Table,
    Cards,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteTypeFilter {
    All,
    Nginx,
    Mysql,
    Postgresql,
}

impl SiteTypeFilter {
    pub fn label(&self) -> &'static str {
        match self {
            SiteTypeFilter::All => "All",
            SiteTypeFilter::Nginx => "Nginx",
            SiteTypeFilter::Mysql => "MySQL",
            SiteTypeFilter::Postgresql => "PostgreSQL",
        }
    }

    fn matches(&self, site: &ManagedSite) -> bool {
        match self {
            SiteTypeFilter::All => true,
            SiteTypeFilter::Nginx => matches!(site.site_type, ManagedSiteType::NginxSite(_)),
            SiteTypeFilter::Mysql => matches!(&site.site_type,
                ManagedSiteType::Database(d) if d.engine == DatabaseEngine::Mysql),
            SiteTypeFilter::Postgresql => matches!(&site.site_type,
                ManagedSiteType::Database(d) if d.engine == DatabaseEngine::Postgresql),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteSortBy {
    Name,
    Server,
    Type,
    DiscoveredAt,
    Status,
}

impl SiteSortBy {
    pub fn label(&self) -> &'static str {
        match self {
            SiteSortBy::Name => "Name",
            SiteSortBy::Server => "Server",
            SiteSortBy::Type => "Type",
            SiteSortBy::DiscoveredAt => "Date",
            SiteSortBy::Status => "Status",
        }
    }
}

/// A flat item in the grouped list — either a group header or a site row.
enum FlatItem {
    GroupHeader {
        group_key: String,
        name: String,
        type_label: String,
        count: usize,
    },
    SiteRow {
        site_index: usize,
        in_group: bool,
    },
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub struct SitesView {
    pub sites: Vec<ManagedSite>,
    pub connections: Vec<Connection>,
    search_query: String,
    type_filter: SiteTypeFilter,
    server_filter: Option<Uuid>,
    tag_filter: Option<String>,
    sort_by: SiteSortBy,
    sort_ascending: bool,
    view_mode: SitesViewMode,
    selected_site: Option<Uuid>,
    detail_panel_open: bool,
    pub scans_pending: u32,
    visible_card_count: usize,
    collapsed_groups: HashSet<String>,
    focus_handle: FocusHandle,
}

impl EventEmitter<SitesEvent> for SitesView {}

impl SitesView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            sites: Vec::new(),
            connections: Vec::new(),
            search_query: String::new(),
            type_filter: SiteTypeFilter::All,
            server_filter: None,
            tag_filter: None,
            sort_by: SiteSortBy::Name,
            sort_ascending: true,
            view_mode: SitesViewMode::Table,
            selected_site: None,
            detail_panel_open: false,
            scans_pending: 0,
            visible_card_count: PAGE_SIZE,
            collapsed_groups: HashSet::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_sites(&mut self, sites: Vec<ManagedSite>) {
        self.sites = sites;
    }

    pub fn set_connections(&mut self, connections: Vec<Connection>) {
        self.connections = connections;
    }

    // -- Stats helpers -------------------------------------------------------

    pub fn total_sites(&self) -> usize {
        self.sites
            .iter()
            .filter(|s| matches!(s.site_type, ManagedSiteType::NginxSite(_)))
            .count()
    }

    pub fn total_databases(&self) -> usize {
        self.sites
            .iter()
            .filter(|s| matches!(s.site_type, ManagedSiteType::Database(_)))
            .count()
    }

    pub fn servers_scanned(&self) -> usize {
        self.sites.iter().map(|s| s.connection_id).collect::<HashSet<_>>().len()
    }

    pub fn ssl_sites_count(&self) -> usize {
        self.sites.iter().filter(|s| s.has_ssl()).count()
    }

    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .sites
            .iter()
            .flat_map(|s| s.tags.iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    // -- Filtering -----------------------------------------------------------

    fn fuzzy_match(haystack: &str, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let h = haystack.to_lowercase();
        let n = needle.to_lowercase();
        let mut hi = h.chars().peekable();
        for nc in n.chars() {
            loop {
                match hi.next() {
                    Some(hc) if hc == nc => break,
                    Some(_) => continue,
                    None => return false,
                }
            }
        }
        true
    }

    fn filtered_sites(&self) -> Vec<&ManagedSite> {
        let mut result: Vec<&ManagedSite> = self
            .sites
            .iter()
            .filter(|s| self.type_filter.matches(s))
            .filter(|s| {
                if let Some(server_id) = self.server_filter {
                    s.connection_id == server_id
                } else {
                    true
                }
            })
            .filter(|s| {
                if let Some(ref tag) = self.tag_filter {
                    s.tags.contains(tag)
                } else {
                    true
                }
            })
            .filter(|s| {
                if self.search_query.is_empty() {
                    true
                } else {
                    Self::fuzzy_match(s.name(), &self.search_query)
                        || Self::fuzzy_match(&s.connection_name, &self.search_query)
                }
            })
            .collect();

        // Favorites first
        result.sort_by_key(|s| std::cmp::Reverse(s.favorite));

        // Then by sort_by
        result.sort_by(|a, b| {
            let cmp = match self.sort_by {
                SiteSortBy::Name => a.name().to_lowercase().cmp(&b.name().to_lowercase()),
                SiteSortBy::Server => a.connection_name.to_lowercase().cmp(&b.connection_name.to_lowercase()),
                SiteSortBy::Type => a.site_type.label().cmp(b.site_type.label()),
                SiteSortBy::DiscoveredAt => a.discovered_at.cmp(&b.discovered_at),
                SiteSortBy::Status => a.status.label().cmp(b.status.label()),
            };
            // Favorites always first regardless of sort
            let fav_cmp = b.favorite.cmp(&a.favorite);
            fav_cmp.then(if self.sort_ascending { cmp } else { cmp.reverse() })
        });

        result
    }

    fn grouped_flat_items(filtered: &[&ManagedSite], collapsed: &HashSet<String>) -> Vec<FlatItem> {
        let mut items = Vec::new();
        let mut i = 0;
        while i < filtered.len() {
            let name = filtered[i].name().to_lowercase();
            let type_label = filtered[i].site_type.label().to_string();
            let group_key = format!("{}|{}", name, type_label);

            let mut count = 1;
            while i + count < filtered.len()
                && filtered[i + count].name().to_lowercase() == name
                && filtered[i + count].site_type.label() == type_label
            {
                count += 1;
            }

            if count > 1 {
                items.push(FlatItem::GroupHeader {
                    group_key: group_key.clone(),
                    name: filtered[i].name().to_string(),
                    type_label,
                    count,
                });
                if !collapsed.contains(&group_key) {
                    for j in 0..count {
                        items.push(FlatItem::SiteRow { site_index: i + j, in_group: true });
                    }
                }
            } else {
                items.push(FlatItem::SiteRow { site_index: i, in_group: false });
            }

            i += count;
        }
        items
    }

    // -- Key handling --------------------------------------------------------

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        match key {
            "escape" => {
                if self.detail_panel_open {
                    self.detail_panel_open = false;
                    self.selected_site = None;
                } else if !self.search_query.is_empty() {
                    self.search_query.clear();
                }
                cx.notify();
                return;
            }
            "backspace" => {
                self.search_query.pop();
                self.visible_card_count = PAGE_SIZE;
                self.collapsed_groups.clear();
                cx.notify();
                return;
            }
            _ => {}
        }

        // Ctrl+V paste
        if key == "v" && mods.secondary() {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    let clean: String = text.lines().next().unwrap_or("").trim().to_string();
                    self.search_query.push_str(&clean);
                    self.visible_card_count = PAGE_SIZE;
                    self.collapsed_groups.clear();
                    cx.notify();
                }
            }
            return;
        }

        // Printable characters
        if let Some(ref kc) = event.keystroke.key_char {
            if !mods.control && !mods.alt {
                self.search_query.push_str(kc);
                self.visible_card_count = PAGE_SIZE;
                self.collapsed_groups.clear();
                cx.notify();
                return;
            }
        }

        if key.len() == 1 && !mods.control && !mods.alt {
            self.search_query.push_str(key);
            self.visible_card_count = PAGE_SIZE;
            self.collapsed_groups.clear();
            cx.notify();
        }
    }

    fn toggle_group_collapse(&mut self, group_key: String, cx: &mut Context<Self>) {
        if !self.collapsed_groups.remove(&group_key) {
            self.collapsed_groups.insert(group_key);
        }
        cx.notify();
    }

    fn render_group_header_row(
        &self,
        group_key: &str,
        name: &str,
        type_label: &str,
        count: usize,
        is_collapsed: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = group_key.to_string();
        let toggle_char = if is_collapsed { ">" } else { "v" };

        let type_color = match type_label {
            "Nginx" => ShellDeckColors::success(),
            "MySQL" => ShellDeckColors::primary(),
            "PostgreSQL" => ShellDeckColors::status_connected(),
            _ => ShellDeckColors::primary(),
        };

        div()
            .id(ElementId::from(SharedString::from(format!("group-hdr-{}", group_key))))
            .flex()
            .items_center()
            .w_full()
            .px(px(12.0))
            .py(px(5.0))
            .bg(ShellDeckColors::bg_surface())
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_group_collapse(key.clone(), cx);
            }))
            .child(
                div()
                    .w(px(16.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(toggle_char),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .mr(px(8.0))
                    .child(name.to_string()),
            )
            .child(
                div()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(type_color.opacity(0.15))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(type_color)
                    .mr(px(8.0))
                    .child(type_label.to_string()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("on {} servers", count)),
            )
    }

    // -- Render helpers ------------------------------------------------------

    fn render_stat_card(label: &str, value: String, accent: Hsla) -> Div {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(120.0))
            .px(px(16.0))
            .py(px(12.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(label.to_string()),
            )
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(accent)
                    .mt(px(4.0))
                    .child(value),
            )
    }

    fn render_stats_row(&self) -> Div {
        div()
            .flex()
            .gap(px(12.0))
            .w_full()
            .flex_shrink_0()
            .child(Self::render_stat_card(
                "SITES",
                self.total_sites().to_string(),
                ShellDeckColors::primary(),
            ))
            .child(Self::render_stat_card(
                "DATABASES",
                self.total_databases().to_string(),
                ShellDeckColors::success(),
            ))
            .child(Self::render_stat_card(
                "SERVERS",
                self.servers_scanned().to_string(),
                ShellDeckColors::warning(),
            ))
            .child(Self::render_stat_card(
                "SSL SITES",
                self.ssl_sites_count().to_string(),
                ShellDeckColors::status_connected(),
            ))
    }

    fn render_filter_badge(
        &self,
        filter: SiteTypeFilter,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let is_active = self.type_filter == filter;
        let label = filter.label();
        div()
            .id(ElementId::from(SharedString::from(format!("filter-{}", label))))
            .px(px(10.0))
            .py(px(4.0))
            .rounded(px(12.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .when(is_active, |el| {
                el.bg(ShellDeckColors::primary())
                    .text_color(gpui::white())
            })
            .when(!is_active, |el| {
                el.bg(ShellDeckColors::bg_surface())
                    .text_color(ShellDeckColors::text_muted())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.type_filter = filter;
                this.visible_card_count = PAGE_SIZE;
                this.collapsed_groups.clear();
                cx.notify();
            }))
            .child(label.to_string())
    }

    fn render_sort_option(
        &self,
        sort: SiteSortBy,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let is_active = self.sort_by == sort;
        let arrow = if is_active {
            if self.sort_ascending { " ^" } else { " v" }
        } else {
            ""
        };
        div()
            .id(ElementId::from(SharedString::from(format!("sort-{}", sort.label()))))
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_size(px(10.0))
            .when(is_active, |el| {
                el.text_color(ShellDeckColors::primary())
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!is_active, |el| {
                el.text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.sort_by == sort {
                    this.sort_ascending = !this.sort_ascending;
                } else {
                    this.sort_by = sort;
                    this.sort_ascending = true;
                }
                this.visible_card_count = PAGE_SIZE;
                this.collapsed_groups.clear();
                cx.notify();
            }))
            .child(format!("{}{}", sort.label(), arrow))
    }

    fn render_filter_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let mut toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .w_full()
            .flex_shrink_0()
            .flex_wrap();

        // Search input display
        let search_display = if self.search_query.is_empty() {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::border())
                .min_w(px(180.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("/ Filter sites..."),
                )
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::primary())
                .min_w(px(180.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(self.search_query.clone()),
                )
                .child(
                    div()
                        .w(px(1.0))
                        .h(px(14.0))
                        .bg(ShellDeckColors::primary()),
                )
        };

        toolbar = toolbar.child(search_display);

        // Type filter badges
        toolbar = toolbar
            .child(
                div()
                    .w(px(1.0))
                    .h(px(20.0))
                    .bg(ShellDeckColors::border()),
            )
            .child(self.render_filter_badge(SiteTypeFilter::All, cx))
            .child(self.render_filter_badge(SiteTypeFilter::Nginx, cx))
            .child(self.render_filter_badge(SiteTypeFilter::Mysql, cx))
            .child(self.render_filter_badge(SiteTypeFilter::Postgresql, cx));

        // Sort options
        toolbar = toolbar.child(
            div()
                .w(px(1.0))
                .h(px(20.0))
                .bg(ShellDeckColors::border()),
        );
        toolbar = toolbar
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Sort:"),
            )
            .child(self.render_sort_option(SiteSortBy::Name, cx))
            .child(self.render_sort_option(SiteSortBy::Server, cx))
            .child(self.render_sort_option(SiteSortBy::Type, cx))
            .child(self.render_sort_option(SiteSortBy::DiscoveredAt, cx));

        // View mode toggle
        toolbar = toolbar.child(
            div()
                .w(px(1.0))
                .h(px(20.0))
                .bg(ShellDeckColors::border()),
        );

        let is_table = self.view_mode == SitesViewMode::Table;
        toolbar = toolbar.child(
            div()
                .flex()
                .items_center()
                .gap(px(2.0))
                .child(
                    div()
                        .id("view-mode-table")
                        .px(px(6.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .when(is_table, |el| {
                            el.bg(ShellDeckColors::primary().opacity(0.15))
                                .text_color(ShellDeckColors::primary())
                        })
                        .when(!is_table, |el| {
                            el.text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view_mode = SitesViewMode::Table;
                            cx.notify();
                        }))
                        .child("Table"),
                )
                .child(
                    div()
                        .id("view-mode-cards")
                        .px(px(6.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .when(!is_table, |el| {
                            el.bg(ShellDeckColors::primary().opacity(0.15))
                                .text_color(ShellDeckColors::primary())
                        })
                        .when(is_table, |el| {
                            el.text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view_mode = SitesViewMode::Cards;
                            cx.notify();
                        }))
                        .child("Cards"),
                ),
        );

        toolbar
    }

    fn render_table_header() -> Div {
        div()
            .flex()
            .items_center()
            .w_full()
            .px(px(12.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .text_size(px(10.0))
            .font_weight(FontWeight::BOLD)
            .text_color(ShellDeckColors::text_muted())
            .child(div().w(px(60.0)).flex_shrink_0().child("Type"))
            .child(div().flex_1().min_w(px(100.0)).child("Name"))
            .child(div().w(px(140.0)).flex_shrink_0().child("Server"))
            .child(div().w(px(60.0)).flex_shrink_0().child("Port"))
            .child(div().w(px(40.0)).flex_shrink_0().child("SSL"))
            .child(div().flex_1().min_w(px(100.0)).child("Root / Size"))
            .child(div().w(px(120.0)).flex_shrink_0().child("Tags"))
            .child(div().w(px(80.0)).flex_shrink_0().child("Actions"))
    }

    fn render_table_row(&self, site: &ManagedSite, cx: &mut Context<Self>) -> Stateful<Div> {
        let site_id = site.id;
        let is_selected = self.selected_site == Some(site_id);
        let group_name = SharedString::from(format!("site-row-{}", site_id));

        let type_color = match &site.site_type {
            ManagedSiteType::NginxSite(_) => ShellDeckColors::success(),
            ManagedSiteType::Database(d) => match d.engine {
                DatabaseEngine::Mysql => ShellDeckColors::primary(),
                DatabaseEngine::Postgresql => ShellDeckColors::status_connected(),
            },
        };

        let port_str = site.port().map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
        let ssl_str = if site.has_ssl() { "[SSL]" } else { "-" };

        let mut tag_row = div().flex().items_center().gap(px(3.0)).w(px(120.0)).flex_shrink_0().overflow_hidden();
        for tag in site.tags.iter().take(2) {
            tag_row = tag_row.child(
                div()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::badge_bg())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(tag.clone()),
            );
        }
        if site.tags.len() > 2 {
            tag_row = tag_row.child(
                div()
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("+{}", site.tags.len() - 2)),
            );
        }

        // Action buttons (hover-reveal)
        let url_for_open = site.url();
        let conn_id = site.connection_id;

        let mut actions = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .w(px(80.0))
            .flex_shrink_0()
            .opacity(0.0)
            .group_hover(group_name.clone(), |el| el.opacity(1.0));

        if let Some(url) = url_for_open {
            actions = actions.child(
                div()
                    .id(ElementId::from(SharedString::from(format!("open-{}", site_id))))
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
                    .on_click(cx.listener(move |_this, _, _, cx| {
                        cx.emit(SitesEvent::OpenInBrowser(url.clone()));
                    }))
                    .child("Open"),
            );
        }

        actions = actions.child(
            div()
                .id(ElementId::from(SharedString::from(format!("ssh-{}", site_id))))
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
                .on_click(cx.listener(move |_this, _, _, cx| {
                    cx.emit(SitesEvent::SshToServer(conn_id));
                }))
                .child("SSH"),
        );

        let fav_label = if site.favorite { "*" } else { "+" };
        actions = actions.child(
            div()
                .id(ElementId::from(SharedString::from(format!("fav-{}", site_id))))
                .px(px(4.0))
                .py(px(2.0))
                .rounded(px(3.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(ShellDeckColors::warning().opacity(0.15))
                        .text_color(ShellDeckColors::warning())
                })
                .on_click(cx.listener(move |_this, _, _, cx| {
                    cx.emit(SitesEvent::ToggleFavorite(site_id));
                }))
                .child(fav_label),
        );

        div()
            .id(ElementId::from(SharedString::from(format!("site-row-click-{}", site_id))))
            .group(group_name)
            .flex()
            .items_center()
            .w_full()
            .px(px(12.0))
            .py(px(5.0))
            .cursor_pointer()
            .when(is_selected, |el| el.bg(ShellDeckColors::selected_bg()))
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_site = Some(site_id);
                this.detail_panel_open = true;
                cx.notify();
            }))
            // Type badge
            .child(
                div()
                    .w(px(60.0))
                    .flex_shrink_0()
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(type_color.opacity(0.15))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(type_color)
                            .child(site.site_type.label().to_string()),
                    ),
            )
            // Name
            .child(
                div()
                    .flex_1()
                    .min_w(px(100.0))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child({
                        let prefix = if site.favorite { "* " } else { "" };
                        format!("{}{}", prefix, site.name())
                    }),
            )
            // Server
            .child(
                div()
                    .w(px(140.0))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(site.connection_name.clone()),
            )
            // Port
            .child(
                div()
                    .w(px(60.0))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(port_str),
            )
            // SSL
            .child(
                div()
                    .w(px(40.0))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(if site.has_ssl() {
                        ShellDeckColors::success()
                    } else {
                        ShellDeckColors::text_muted()
                    })
                    .child(ssl_str),
            )
            // Root/Size
            .child(
                div()
                    .flex_1()
                    .min_w(px(100.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(site.root_or_size()),
            )
            // Tags
            .child(tag_row)
            // Actions
            .child(actions)
    }

    fn render_card(&self, site: &ManagedSite, cx: &mut Context<Self>) -> Stateful<Div> {
        let site_id = site.id;
        let is_selected = self.selected_site == Some(site_id);

        let type_color = match &site.site_type {
            ManagedSiteType::NginxSite(_) => ShellDeckColors::success(),
            ManagedSiteType::Database(d) => match d.engine {
                DatabaseEngine::Mysql => ShellDeckColors::primary(),
                DatabaseEngine::Postgresql => ShellDeckColors::status_connected(),
            },
        };

        let mut card = div()
            .id(ElementId::from(SharedString::from(format!("site-card-{}", site_id))))
            .flex()
            .flex_col()
            .w(px(220.0))
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .cursor_pointer()
            .when(is_selected, |el| el.border_color(ShellDeckColors::primary()))
            .when(!is_selected, |el| {
                el.border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::primary().opacity(0.5)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_site = Some(site_id);
                this.detail_panel_open = true;
                cx.notify();
            }));

        // Type badge + favorite star
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .w(px(24.0))
                                .h(px(24.0))
                                .rounded(px(6.0))
                                .bg(type_color.opacity(0.15))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(type_color)
                                .child(site.site_type.type_icon().to_string()),
                        )
                        .child(
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(3.0))
                                .bg(type_color.opacity(0.1))
                                .text_size(px(9.0))
                                .text_color(type_color)
                                .child(site.site_type.label().to_string()),
                        ),
                )
                .when(site.favorite, |el| {
                    el.child(
                        div()
                            .text_size(px(14.0))
                            .text_color(ShellDeckColors::warning())
                            .child("*"),
                    )
                }),
        );

        // Name
        card = card.child(
            div()
                .mt(px(8.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::text_primary())
                .overflow_hidden()
                .whitespace_nowrap()
                .child(site.name().to_string()),
        );

        // Server name
        card = card.child(
            div()
                .mt(px(2.0))
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .overflow_hidden()
                .whitespace_nowrap()
                .child(site.connection_name.clone()),
        );

        // Port + SSL badges
        let mut badges = div().flex().items_center().gap(px(4.0)).mt(px(6.0));
        if let Some(port) = site.port() {
            badges = badges.child(
                div()
                    .px(px(5.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::badge_bg())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(":{}", port)),
            );
        }
        if site.has_ssl() {
            badges = badges.child(
                div()
                    .px(px(5.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::success().opacity(0.15))
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::success())
                    .child("SSL"),
            );
        }
        // Tags
        for tag in site.tags.iter().take(2) {
            badges = badges.child(
                div()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::badge_bg())
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(tag.clone()),
            );
        }
        card = card.child(badges);

        card
    }

    fn render_detail_panel(&self, cx: &mut Context<Self>) -> Div {
        let site_id = match self.selected_site {
            Some(id) => id,
            None => return div(),
        };

        let site = match self.sites.iter().find(|s| s.id == site_id) {
            Some(s) => s,
            None => return div(),
        };

        let type_color = match &site.site_type {
            ManagedSiteType::NginxSite(_) => ShellDeckColors::success(),
            ManagedSiteType::Database(d) => match d.engine {
                DatabaseEngine::Mysql => ShellDeckColors::primary(),
                DatabaseEngine::Postgresql => ShellDeckColors::status_connected(),
            },
        };

        let fav_id = site.id;
        let remove_id = site.id;
        let conn_id = site.connection_id;
        let url_for_open = site.url();

        let mut panel = div()
            .flex()
            .flex_col()
            .w(px(320.0))
            .flex_shrink_0()
            .h_full()
            .border_l_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .overflow_hidden();

        // Header
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(16.0))
                .py(px(12.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(
                            div()
                                .w(px(28.0))
                                .h(px(28.0))
                                .flex_shrink_0()
                                .rounded(px(6.0))
                                .bg(type_color.opacity(0.15))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(type_color)
                                .child(site.site_type.type_icon().to_string()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(site.name().to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(type_color)
                                        .child(site.site_type.label().to_string()),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("detail-close")
                        .flex_shrink_0()
                        .px(px(6.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.detail_panel_open = false;
                            this.selected_site = None;
                            cx.notify();
                        }))
                        .child("x"),
                ),
        );

        // Scrollable content
        let mut content = div()
            .id("detail-content")
            .flex()
            .flex_col()
            .flex_grow()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .px(px(16.0))
            .py(px(12.0))
            .gap(px(10.0));

        // Server info
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(Self::detail_label("Server"))
                .child(Self::detail_value(&site.connection_name)),
        );

        // Type-specific details
        match &site.site_type {
            ManagedSiteType::NginxSite(s) => {
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(Self::detail_label("Config Path"))
                        .child(Self::detail_value(&s.config_path)),
                );
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(Self::detail_label("Root"))
                        .child(Self::detail_value(&s.root)),
                );
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(Self::detail_label("Port"))
                                .child(Self::detail_value(&s.listen_port.to_string())),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(Self::detail_label("SSL"))
                                .child(Self::detail_value(if s.ssl { "Yes" } else { "No" })),
                        ),
                );
                if let Some(ref url) = site.url() {
                    content = content.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(Self::detail_label("URL"))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::primary())
                                    .child(url.clone()),
                            ),
                    );
                }
            }
            ManagedSiteType::Database(d) => {
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(Self::detail_label("Engine"))
                        .child(Self::detail_value(d.engine.label())),
                );
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(Self::detail_label("Size"))
                        .child(Self::detail_value(&d.size_display())),
                );
                if let Some(tc) = d.table_count {
                    content = content.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(Self::detail_label("Tables"))
                            .child(Self::detail_value(&tc.to_string())),
                    );
                }
            }
        }

        // Tags
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(Self::detail_label("Tags"))
                .child(if site.tags.is_empty() {
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("No tags")
                } else {
                    let mut tag_row = div().flex().items_center().gap(px(4.0)).flex_wrap();
                    for tag in &site.tags {
                        tag_row = tag_row.child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::badge_bg())
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(tag.clone()),
                        );
                    }
                    tag_row
                }),
        );

        // Notes
        if let Some(ref notes) = site.notes {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(Self::detail_label("Notes"))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(notes.clone()),
                    ),
            );
        }

        // Status
        let status_color = match &site.status {
            SiteStatus::Unknown => ShellDeckColors::text_muted(),
            SiteStatus::Online => ShellDeckColors::success(),
            SiteStatus::Offline => ShellDeckColors::error(),
            SiteStatus::Error(_) => ShellDeckColors::error(),
        };
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(Self::detail_label("Status"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .rounded_full()
                                .bg(status_color),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(status_color)
                                .child(match &site.status {
                                    SiteStatus::Error(msg) => format!("Error: {}", msg),
                                    other => other.label().to_string(),
                                }),
                        ),
                ),
        );

        // Discovered at
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(Self::detail_label("Discovered"))
                .child(Self::detail_value(&site.discovered_at.format("%Y-%m-%d %H:%M").to_string())),
        );

        panel = panel.child(content);

        // Quick actions footer
        let mut actions = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .flex_shrink_0()
            .px(px(16.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(ShellDeckColors::border());

        if let Some(url) = url_for_open {
            actions = actions.child(Self::detail_action_button(
                "detail-open-browser",
                "Open in Browser",
                ShellDeckColors::primary(),
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(SitesEvent::OpenInBrowser(url.clone()));
                }),
            ));
        }

        actions = actions.child(Self::detail_action_button(
            "detail-ssh",
            "SSH to Server",
            ShellDeckColors::success(),
            cx.listener(move |_this, _, _, cx| {
                cx.emit(SitesEvent::SshToServer(conn_id));
            }),
        ));

        let check_id = site.id;
        actions = actions.child(Self::detail_action_button(
            "detail-check-status",
            "Check Status",
            ShellDeckColors::status_connected(),
            cx.listener(move |_this, _, _, cx| {
                cx.emit(SitesEvent::CheckSiteStatus(check_id));
            }),
        ));

        actions = actions.child(Self::detail_action_button(
            "detail-fav",
            if site.favorite { "Remove Favorite" } else { "Add Favorite" },
            ShellDeckColors::warning(),
            cx.listener(move |_this, _, _, cx| {
                cx.emit(SitesEvent::ToggleFavorite(fav_id));
            }),
        ));

        let sync_id = site.id;
        actions = actions.child(Self::detail_action_button(
            "detail-add-sync",
            "Add to Sync",
            ShellDeckColors::primary(),
            cx.listener(move |_this, _, _, cx| {
                cx.emit(SitesEvent::AddToSync(sync_id));
            }),
        ));

        actions = actions.child(Self::detail_action_button(
            "detail-remove",
            "Remove Site",
            ShellDeckColors::error(),
            cx.listener(move |_this, _, _, cx| {
                cx.emit(SitesEvent::RemoveSite(remove_id));
            }),
        ));

        panel = panel.child(actions);

        panel
    }

    fn detail_label(text: &str) -> Div {
        div()
            .text_size(px(10.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .child(text.to_uppercase())
    }

    fn detail_value(text: &str) -> Div {
        div()
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_primary())
            .child(text.to_string())
    }

    fn detail_action_button(
        id: &str,
        label: &str,
        color: Hsla,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(ElementId::from(SharedString::from(id.to_string())))
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .py(px(5.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .bg(color.opacity(0.1))
            .hover(|el| el.bg(color.opacity(0.2)))
            .on_click(handler)
            .child(label.to_string())
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .flex_grow()
            .gap(px(12.0))
            .child(
                div()
                    .w(px(48.0))
                    .h(px(48.0))
                    .rounded(px(12.0))
                    .bg(ShellDeckColors::primary().opacity(0.1))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(20.0))
                    .text_color(ShellDeckColors::primary())
                    .child("W"),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child("No sites discovered"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Scan your servers to discover sites and databases"),
            )
            .child(
                div()
                    .id("empty-scan-btn")
                    .mt(px(8.0))
                    .px(px(16.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_color(gpui::white())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|el| el.opacity(0.9))
                    .on_click(cx.listener(|_this, _, _, cx| {
                        cx.emit(SitesEvent::ScanAllServers);
                    }))
                    .child("Scan All Servers"),
            )
    }
}

impl Render for SitesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered = self.filtered_sites();
        let flat_items_count = Self::grouped_flat_items(&filtered, &self.collapsed_groups).len();
        let has_sites = !self.sites.is_empty();
        let has_filtered = !filtered.is_empty();

        let mut page = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .id("sites-view")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key_down(event, cx);
            }));

        // Page header
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_shrink_0()
            .px(px(24.0))
            .py(px(16.0))
            .border_b_1()
            .border_color(ShellDeckColors::border());

        header = header.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child("Sites"),
                )
                .when(self.scans_pending > 0, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded_full()
                                    .bg(ShellDeckColors::success()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!("Scanning ({} remaining)...", self.scans_pending)),
                            ),
                    )
                }),
        );

        let scanning = self.scans_pending > 0;
        let mut scan_btn = div()
            .id("scan-all-btn")
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM);

        if scanning {
            scan_btn = scan_btn
                .bg(ShellDeckColors::primary().opacity(0.4))
                .text_color(gpui::white().opacity(0.6));
        } else {
            scan_btn = scan_btn
                .bg(ShellDeckColors::primary())
                .text_color(gpui::white())
                .cursor_pointer()
                .hover(|el| el.opacity(0.9))
                .on_click(cx.listener(|_this, _, _, cx| {
                    cx.emit(SitesEvent::ScanAllServers);
                }));
        }
        scan_btn = scan_btn.child("Scan All Servers");

        let clear_btn = div()
            .id("clear-all-sites-btn")
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .bg(ShellDeckColors::error().opacity(0.15))
            .text_color(ShellDeckColors::error())
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::error().opacity(0.25)))
            .on_click(cx.listener(|_this, _, _, cx| {
                cx.emit(SitesEvent::ClearAllSites);
            }))
            .child("Clear All");

        header = header.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(clear_btn)
                .child(scan_btn),
        );

        page = page.child(header);

        if !has_sites {
            page = page.child(self.render_empty_state(cx));
            return page;
        }

        // Stats row
        page = page.child(
            div()
                .px(px(24.0))
                .py(px(12.0))
                .flex_shrink_0()
                .child(self.render_stats_row()),
        );

        // Filter toolbar
        page = page.child(
            div()
                .px(px(24.0))
                .py(px(8.0))
                .flex_shrink_0()
                .child(self.render_filter_toolbar(cx)),
        );

        // Result count line
        let filtered_count = filtered.len();
        let total_count = self.sites.len();
        let count_text = if filtered_count == total_count {
            format!("{} sites", total_count)
        } else {
            format!("{} of {} sites", filtered_count, total_count)
        };
        page = page.child(
            div()
                .px(px(24.0))
                .pb(px(4.0))
                .flex_shrink_0()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(count_text),
                ),
        );

        // Content area (table/cards + optional detail panel)
        let mut content_area = div()
            .flex()
            .flex_grow()
            .min_h(px(0.0))
            .overflow_hidden();

        // Main list area
        let mut list_area = div()
            .flex()
            .flex_col()
            .flex_grow()
            .min_h(px(0.0))
            .overflow_hidden();

        if !has_filtered {
            list_area = list_area.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_grow()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("No sites match current filters"),
            );
        } else {
            match self.view_mode {
                SitesViewMode::Table => {
                    list_area = list_area.child(Self::render_table_header());

                    let table = uniform_list(
                        "sites-table-body",
                        flat_items_count,
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            let filtered = this.filtered_sites();
                            let flat_items = Self::grouped_flat_items(&filtered, &this.collapsed_groups);
                            let mut items: Vec<AnyElement> = Vec::new();
                            for i in range {
                                if let Some(flat_item) = flat_items.get(i) {
                                    match flat_item {
                                        FlatItem::GroupHeader { group_key, name, type_label, count } => {
                                            let is_collapsed = this.collapsed_groups.contains(group_key);
                                            items.push(
                                                this.render_group_header_row(
                                                    group_key, name, type_label, *count, is_collapsed, cx,
                                                )
                                                .into_any_element(),
                                            );
                                        }
                                        FlatItem::SiteRow { site_index, in_group } => {
                                            if let Some(site) = filtered.get(*site_index) {
                                                let row = this.render_table_row(site, cx);
                                                if *in_group {
                                                    items.push(
                                                        div()
                                                            .id(ElementId::from(SharedString::from(
                                                                format!("grp-row-{}", site_index),
                                                            )))
                                                            .w_full()
                                                            .border_l_2()
                                                            .border_color(
                                                                ShellDeckColors::primary().opacity(0.2),
                                                            )
                                                            .child(row)
                                                            .into_any_element(),
                                                    );
                                                } else {
                                                    items.push(row.into_any_element());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            items
                        }),
                    )
                    .flex_grow()
                    .min_h(px(0.));

                    list_area = list_area.child(table);
                }
                SitesViewMode::Cards => {
                    let visible_count = self.visible_card_count;

                    let mut card_grid = div()
                        .id("sites-card-grid")
                        .flex()
                        .flex_col()
                        .flex_grow()
                        .min_h(px(0.0))
                        .overflow_y_scroll();

                    let mut cards_wrap = div()
                        .flex()
                        .flex_wrap()
                        .gap(px(12.0))
                        .p(px(24.0));

                    let mut card_i = 0;
                    let mut site_cards_shown: usize = 0;
                    while card_i < filtered.len() && site_cards_shown < visible_count {
                        let gname = filtered[card_i].name().to_lowercase();
                        let gtype = filtered[card_i].site_type.label().to_string();

                        let mut group_count = 1;
                        while card_i + group_count < filtered.len()
                            && filtered[card_i + group_count].name().to_lowercase() == gname
                            && filtered[card_i + group_count].site_type.label() == gtype
                        {
                            group_count += 1;
                        }

                        if group_count > 1 {
                            let type_color = match gtype.as_str() {
                                "Nginx" => ShellDeckColors::success(),
                                "MySQL" => ShellDeckColors::primary(),
                                "PostgreSQL" => ShellDeckColors::status_connected(),
                                _ => ShellDeckColors::primary(),
                            };
                            cards_wrap = cards_wrap.child(
                                div()
                                    .w_full()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .px(px(4.0))
                                    .py(px(6.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(filtered[card_i].name().to_string()),
                                    )
                                    .child(
                                        div()
                                            .px(px(6.0))
                                            .py(px(2.0))
                                            .rounded(px(4.0))
                                            .bg(type_color.opacity(0.15))
                                            .text_size(px(10.0))
                                            .text_color(type_color)
                                            .child(gtype.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(format!("{} servers", group_count)),
                                    ),
                            );
                            for j in 0..group_count {
                                if site_cards_shown >= visible_count {
                                    break;
                                }
                                cards_wrap = cards_wrap.child(self.render_card(filtered[card_i + j], cx));
                                site_cards_shown += 1;
                            }
                        } else {
                            cards_wrap = cards_wrap.child(self.render_card(filtered[card_i], cx));
                            site_cards_shown += 1;
                        }

                        card_i += group_count;
                    }

                    card_grid = card_grid.child(cards_wrap);

                    // Pagination footer
                    if site_cards_shown < filtered_count {
                        let showing = site_cards_shown;
                        card_grid = card_grid.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(12.0))
                                .px(px(24.0))
                                .py(px(12.0))
                                .flex_shrink_0()
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(format!("Showing {} of {}", showing, filtered_count)),
                                )
                                .child(
                                    div()
                                        .id("show-more-btn")
                                        .px(px(12.0))
                                        .py(px(5.0))
                                        .rounded(px(6.0))
                                        .bg(ShellDeckColors::primary())
                                        .text_color(gpui::white())
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .cursor_pointer()
                                        .hover(|el| el.opacity(0.9))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.visible_card_count += PAGE_SIZE;
                                            cx.notify();
                                        }))
                                        .child("Show More"),
                                )
                                .child(
                                    div()
                                        .id("show-all-btn")
                                        .px(px(12.0))
                                        .py(px(5.0))
                                        .rounded(px(6.0))
                                        .bg(ShellDeckColors::bg_surface())
                                        .border_1()
                                        .border_color(ShellDeckColors::border())
                                        .text_color(ShellDeckColors::text_muted())
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .cursor_pointer()
                                        .hover(|el| {
                                            el.bg(ShellDeckColors::hover_bg())
                                                .text_color(ShellDeckColors::text_primary())
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.visible_card_count = usize::MAX;
                                            cx.notify();
                                        }))
                                        .child("Show All"),
                                ),
                        );
                    }

                    list_area = list_area.child(card_grid);
                }
            }
        }

        content_area = content_area.child(list_area);

        // Detail panel
        if self.detail_panel_open && self.selected_site.is_some() {
            content_area = content_area.child(self.render_detail_panel(cx));
        }

        page = page.child(content_area);

        page
    }
}
