use crate::scale::px;
use adabraka_ui::components::select::{Select, SelectOption};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::Connection;
use shelldeck_core::models::server_sync::*;
use uuid::Uuid;

use crate::theme::ShellDeckColors;
use crate::t;

// Theme helpers — map semantic names to existing ShellDeckColors methods.
fn bg_secondary() -> gpui::Hsla {
    ShellDeckColors::bg_surface()
}
fn bg_tertiary() -> gpui::Hsla {
    ShellDeckColors::bg_sidebar()
}

fn local_machine_label() -> String {
    t!("sync.local_machine").to_string()
}

/// Which side of the split view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSide {
    Source,
    Destination,
}

/// Events emitted by ServerSyncView to be handled by Workspace.
#[derive(Debug, Clone)]
pub enum ServerSyncEvent {
    DiscoverServices {
        connection_id: Uuid,
        panel: PanelSide,
    },
    ListFiles {
        connection_id: Uuid,
        path: String,
        panel: PanelSide,
    },
    StartSync(SyncProfile),
    CancelSync(Uuid),
    SaveProfile(SyncProfile),
    DeleteProfile(Uuid),
    ExecSync {
        source_connection_id: Uuid,
        command: String,
        operation_id: Uuid,
        item_id: Uuid,
    },
}

/// Wizard steps for the sync configuration flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    SelectItems,
    ConfigureOptions,
    ReviewConfirm,
    Executing,
}

/// Sentinel UUID for the local machine (all zeros).
pub const LOCAL_MACHINE_ID: Uuid = Uuid::nil();

/// State for one side of the sync panel.
pub struct ServerPanelState {
    pub connection_id: Option<Uuid>,
    pub connection_name: String,
    pub is_local: bool,
    pub current_path: String,
    pub file_entries: Vec<FileEntry>,
    pub path_history: Vec<String>,
    pub discovered_sites: Vec<DiscoveredSite>,
    pub discovered_databases: Vec<DiscoveredDatabase>,
    pub discovery_loading: bool,
    pub files_loading: bool,
    pub discovery_panel_height: f32,
    pub discovery_resizing: bool,
}

impl ServerPanelState {
    fn new() -> Self {
        Self {
            connection_id: None,
            connection_name: String::new(),
            is_local: false,
            current_path: "/".to_string(),
            file_entries: Vec::new(),
            path_history: Vec::new(),
            discovered_sites: Vec::new(),
            discovered_databases: Vec::new(),
            discovery_loading: false,
            files_loading: false,
            discovery_panel_height: 150.0,
            discovery_resizing: false,
        }
    }
}

pub struct ServerSyncView {
    pub source_panel: ServerPanelState,
    pub dest_panel: ServerPanelState,
    pub connections: Vec<Connection>,
    pub profiles: Vec<SyncProfile>,
    pub selected_profile: Option<Uuid>,
    pub wizard_active: bool,
    pub wizard_step: WizardStep,
    pub wizard_items: Vec<SyncItem>,
    pub wizard_options: SyncOptions,
    pub active_operation: Option<SyncOperation>,
    pub log_lines: Vec<String>,
    source_select: Entity<Select<Uuid>>,
    dest_select: Entity<Select<Uuid>>,
    pub log_panel_height: f32,
    pub log_panel_resizing: bool,
    focus_handle: FocusHandle,
    pub panel_ratio: f32,
    pub panel_dragging: bool,
}

impl EventEmitter<ServerSyncEvent> for ServerSyncView {}

impl ServerSyncView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let parent = cx.entity();
        let source_select =
            Self::build_connection_select(PanelSide::Source, &[], None, parent.clone(), cx);
        let dest_select =
            Self::build_connection_select(PanelSide::Destination, &[], None, parent, cx);
        Self {
            source_panel: ServerPanelState::new(),
            dest_panel: ServerPanelState::new(),
            connections: Vec::new(),
            profiles: Vec::new(),
            selected_profile: None,
            wizard_active: false,
            wizard_step: WizardStep::SelectItems,
            wizard_items: Vec::new(),
            wizard_options: SyncOptions::default(),
            active_operation: None,
            log_lines: Vec::new(),
            source_select,
            dest_select,
            log_panel_height: 200.0,
            log_panel_resizing: false,
            focus_handle: cx.focus_handle(),
            panel_ratio: 0.5,
            panel_dragging: false,
        }
    }

    pub fn is_panel_dragging(&self) -> bool {
        self.panel_dragging
    }

    pub fn is_log_resizing(&self) -> bool {
        self.log_panel_resizing
    }

    pub fn is_discovery_resizing(&self) -> bool {
        self.source_panel.discovery_resizing || self.dest_panel.discovery_resizing
    }

    pub fn stop_discovery_resizing(&mut self) {
        self.source_panel.discovery_resizing = false;
        self.dest_panel.discovery_resizing = false;
    }

    pub fn set_connections(&mut self, connections: Vec<Connection>, cx: &mut Context<Self>) {
        self.connections = connections;
        self.refresh_connection_selects(cx);
    }

    pub fn set_profiles(&mut self, profiles: Vec<SyncProfile>) {
        self.profiles = profiles;
    }

    pub fn load_profile(&mut self, profile_id: Uuid, cx: &mut Context<Self>) {
        self.selected_profile = Some(profile_id);
        if let Some(profile) = self.profiles.iter().find(|p| p.id == profile_id) {
            // Set source connection
            if profile.source_connection_id == LOCAL_MACHINE_ID {
                self.source_panel.connection_id = Some(LOCAL_MACHINE_ID);
                self.source_panel.connection_name = local_machine_label();
                self.source_panel.is_local = true;
            } else if let Some(src_conn) = self
                .connections
                .iter()
                .find(|c| c.id == profile.source_connection_id)
            {
                self.source_panel.connection_id = Some(src_conn.id);
                self.source_panel.connection_name = src_conn.display_name().to_string();
                self.source_panel.is_local = false;
            }
            // Set dest connection
            if profile.dest_connection_id == LOCAL_MACHINE_ID {
                self.dest_panel.connection_id = Some(LOCAL_MACHINE_ID);
                self.dest_panel.connection_name = local_machine_label();
                self.dest_panel.is_local = true;
            } else if let Some(dest_conn) = self
                .connections
                .iter()
                .find(|c| c.id == profile.dest_connection_id)
            {
                self.dest_panel.connection_id = Some(dest_conn.id);
                self.dest_panel.connection_name = dest_conn.display_name().to_string();
                self.dest_panel.is_local = false;
            }
            // Load items
            self.wizard_items = profile.items.clone();
            self.wizard_options = profile.options.clone();
        }
        self.refresh_connection_selects(cx);
    }

    pub fn set_file_entries(&mut self, panel: PanelSide, path: String, entries: Vec<FileEntry>) {
        let state = match panel {
            PanelSide::Source => &mut self.source_panel,
            PanelSide::Destination => &mut self.dest_panel,
        };
        state.current_path = path;
        state.file_entries = entries;
        state.files_loading = false;
    }

    pub fn set_discovered_sites(&mut self, panel: PanelSide, sites: Vec<DiscoveredSite>) {
        let state = match panel {
            PanelSide::Source => &mut self.source_panel,
            PanelSide::Destination => &mut self.dest_panel,
        };
        state.discovered_sites = sites;
    }

    pub fn set_discovered_databases(
        &mut self,
        panel: PanelSide,
        databases: Vec<DiscoveredDatabase>,
    ) {
        let state = match panel {
            PanelSide::Source => &mut self.source_panel,
            PanelSide::Destination => &mut self.dest_panel,
        };
        state.discovered_databases = databases;
        state.discovery_loading = false;
    }

    pub fn append_log(&mut self, line: String) {
        self.log_lines.push(line);
    }

    pub fn panel_state_mut(&mut self, side: PanelSide) -> &mut ServerPanelState {
        match side {
            PanelSide::Source => &mut self.source_panel,
            PanelSide::Destination => &mut self.dest_panel,
        }
    }

    fn panel_state(&self, side: PanelSide) -> &ServerPanelState {
        match side {
            PanelSide::Source => &self.source_panel,
            PanelSide::Destination => &self.dest_panel,
        }
    }

    // -----------------------------------------------------------------------
    // Toolbar
    // -----------------------------------------------------------------------
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let both_connected =
            self.source_panel.connection_id.is_some() && self.dest_panel.connection_id.is_some();

        let mut toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(bg_secondary());

        // Left side: title + profile selector
        let mut left = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .flex_grow()
            .min_w(px(0.0))
            .overflow_hidden();
        left = left.child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(t!("sync.title").to_string()),
        );

        // Profile dropdown
        if !self.profiles.is_empty() {
            let mut profile_section = div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .min_w(px(0.0))
                .overflow_hidden();
            for profile in &self.profiles {
                let pid = profile.id;
                let pname = profile.name.clone();
                let is_active = self.selected_profile == Some(pid);
                profile_section = profile_section.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "profile-{}",
                            pid
                        ))))
                        .max_w(px(180.0))
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .border_1()
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .overflow_hidden()
                        .truncate()
                        .when(is_active, |el| {
                            el.border_color(ShellDeckColors::primary())
                                .bg(ShellDeckColors::primary().opacity(0.1))
                                .text_color(ShellDeckColors::primary())
                        })
                        .when(!is_active, |el| {
                            el.border_color(ShellDeckColors::border())
                                .bg(ShellDeckColors::bg_primary())
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.load_profile(pid, cx);
                            cx.notify();
                        }))
                        .child(pname),
                );
            }
            left = left.child(profile_section);
        }

        toolbar = toolbar.child(left);

        // Right side: buttons
        let mut right = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .flex_shrink_0();

        // Save Profile button
        if both_connected {
            right = right.child(
                div()
                    .id("save-profile-btn")
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let (Some(src_id), Some(dest_id)) = (
                            this.source_panel.connection_id,
                            this.dest_panel.connection_id,
                        ) {
                            let profile = SyncProfile {
                                id: this.selected_profile.unwrap_or_else(Uuid::new_v4),
                                name: format!(
                                    "{} -> {}",
                                    this.source_panel.connection_name,
                                    this.dest_panel.connection_name
                                ),
                                description: None,
                                source_connection_id: src_id,
                                dest_connection_id: dest_id,
                                items: this.wizard_items.clone(),
                                options: this.wizard_options.clone(),
                                created_at: chrono::Utc::now(),
                                last_synced: None,
                            };
                            cx.emit(ServerSyncEvent::SaveProfile(profile));
                        }
                        cx.notify();
                    }))
                    .child(t!("sync.save_profile").to_string()),
            );
        }

        // Delete Profile button (only if a profile is selected)
        if let Some(profile_id) = self.selected_profile {
            right = right.child(
                div()
                    .id("delete-profile-btn")
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::error().opacity(0.3))
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .hover(|el| el.bg(ShellDeckColors::error().opacity(0.1)))
                    .on_click(cx.listener(move |_this, _, _, cx| {
                        cx.emit(ServerSyncEvent::DeleteProfile(profile_id));
                    }))
                    .child(t!("sync.delete").to_string()),
            );
        }

        // Start Sync button
        right = right.child(
            div()
                .id("start-sync-btn")
                .px(px(14.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .when(both_connected, |el| {
                    el.bg(ShellDeckColors::primary())
                        .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                        .hover(|el| el.opacity(0.9))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.wizard_active = true;
                            this.wizard_step = WizardStep::SelectItems;
                            this.wizard_items.clear();
                            this.wizard_options = SyncOptions::default();
                            cx.notify();
                        }))
                })
                .when(!both_connected, |el| {
                    el.bg(bg_tertiary())
                        .text_color(ShellDeckColors::text_muted())
                })
                .child(t!("sync.start").to_string()),
        );

        toolbar = toolbar.child(right);
        toolbar
    }

    // -----------------------------------------------------------------------
    // Connection picker (adabraka-ui Select — see .agents/ui-components.md)
    // -----------------------------------------------------------------------
    fn connection_select_options(connections: &[Connection]) -> Vec<SelectOption<Uuid>> {
        let mut options = vec![SelectOption::new(LOCAL_MACHINE_ID, local_machine_label())
            .with_group(t!("sync.group.local").to_string())
            .with_icon("icons/lucide/user.svg")];
        for conn in connections {
            let label = format!("{} — {}", conn.display_name(), conn.hostname);
            options.push(
                SelectOption::new(conn.id, label)
                    .with_group(t!("sync.group.connections").to_string())
                    .with_icon("icons/lucide/server.svg"),
            );
        }
        options
    }

    fn build_connection_select(
        side: PanelSide,
        connections: &[Connection],
        selected_id: Option<Uuid>,
        parent: Entity<ServerSyncView>,
        cx: &mut Context<ServerSyncView>,
    ) -> Entity<Select<Uuid>> {
        let options = Self::connection_select_options(connections);
        let selected_index = selected_id.and_then(|id| options.iter().position(|o| o.value == id));
        let placeholder = match side {
            PanelSide::Source => t!("sync.placeholder.source").to_string(),
            PanelSide::Destination => t!("sync.placeholder.destination").to_string(),
        };
        cx.new(|select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .placeholder(placeholder)
                .searchable(true)
                .on_change({
                    move |conn_id, _window, cx| {
                        parent.update(cx, |view, cx| {
                            view.on_connection_picked(side, *conn_id, cx);
                        });
                    }
                })
        })
    }

    fn refresh_connection_selects(&mut self, cx: &mut Context<Self>) {
        let parent = cx.entity();
        let src_id = self.source_panel.connection_id;
        let dest_id = self.dest_panel.connection_id;
        let conns = self.connections.clone();
        self.source_select = Self::build_connection_select(
            PanelSide::Source,
            &conns,
            src_id,
            parent.clone(),
            cx,
        );
        self.dest_select = Self::build_connection_select(
            PanelSide::Destination,
            &conns,
            dest_id,
            parent,
            cx,
        );
    }

    fn on_connection_picked(&mut self, side: PanelSide, conn_id: Uuid, cx: &mut Context<Self>) {
        let is_local = conn_id == LOCAL_MACHINE_ID;
        let (name, path) = if is_local {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            (local_machine_label(), home)
        } else if let Some(conn) = self.connections.iter().find(|c| c.id == conn_id) {
            (conn.display_name().to_string(), "/".to_string())
        } else {
            return;
        };

        let state = match side {
            PanelSide::Source => &mut self.source_panel,
            PanelSide::Destination => &mut self.dest_panel,
        };
        state.connection_id = Some(conn_id);
        state.connection_name = name;
        state.is_local = is_local;
        state.current_path = path.clone();
        state.file_entries.clear();
        state.discovered_sites.clear();
        state.discovered_databases.clear();
        state.files_loading = true;

        cx.emit(ServerSyncEvent::ListFiles {
            connection_id: conn_id,
            path,
            panel: side,
        });
        cx.notify();
    }

    fn render_connection_picker(
        &self,
        side: PanelSide,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match side {
            PanelSide::Source => self.source_select.clone(),
            PanelSide::Destination => self.dest_select.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Path breadcrumbs
    // -----------------------------------------------------------------------
    fn render_breadcrumbs(&self, side: PanelSide, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.panel_state(side);
        let current = &state.current_path;

        let mut crumbs = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .px(px(10.0))
            .pt(px(4.0))
            .pb(px(6.0))
            .overflow_hidden()
            .w_full();

        // Root
        crumbs = crumbs.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "crumb-root-{:?}",
                    side
                ))))
                .text_size(px(11.0))
                .text_color(ShellDeckColors::primary())
                .cursor_pointer()
                .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let state = match side {
                        PanelSide::Source => &mut this.source_panel,
                        PanelSide::Destination => &mut this.dest_panel,
                    };
                    if let Some(conn_id) = state.connection_id {
                        state.files_loading = true;
                        cx.emit(ServerSyncEvent::ListFiles {
                            connection_id: conn_id,
                            path: "/".to_string(),
                            panel: side,
                        });
                        cx.notify();
                    }
                }))
                .child("/"),
        );

        // Path segments
        let parts: Vec<&str> = current.split('/').filter(|s| !s.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let nav_path = format!("/{}", parts[..=i].join("/"));
            let part_string = part.to_string();
            crumbs = crumbs
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("/"),
                )
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "crumb-{}-{}-{:?}",
                            i, part_string, side
                        ))))
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .when(i == parts.len() - 1, |el| {
                            el.text_color(ShellDeckColors::text_primary())
                                .font_weight(FontWeight::MEDIUM)
                        })
                        .when(i < parts.len() - 1, |el| {
                            el.text_color(ShellDeckColors::primary())
                                .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let state = match side {
                                PanelSide::Source => &mut this.source_panel,
                                PanelSide::Destination => &mut this.dest_panel,
                            };
                            if let Some(conn_id) = state.connection_id {
                                state.files_loading = true;
                                cx.emit(ServerSyncEvent::ListFiles {
                                    connection_id: conn_id,
                                    path: nav_path.clone(),
                                    panel: side,
                                });
                                cx.notify();
                            }
                        }))
                        .child(part_string),
                );
        }

        crumbs
    }

    // -----------------------------------------------------------------------
    // File list
    // -----------------------------------------------------------------------
    fn render_file_list(&self, side: PanelSide, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.panel_state(side);

        let mut list = div()
            .id(ElementId::from(SharedString::from(format!(
                "file-list-{:?}",
                side
            ))))
            .flex()
            .flex_col()
            .flex_grow()
            .overflow_y_scroll()
            .min_h(px(0.0));

        if state.files_loading {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py(px(20.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.loading").to_string()),
            );
            return list;
        }

        if state.connection_id.is_none() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .py(px(40.0))
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(24.0))
                            .text_color(ShellDeckColors::text_muted().opacity(0.4))
                            .child(match side {
                                PanelSide::Source => "@",
                                PanelSide::Destination => "@",
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("sync.select_connection").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted().opacity(0.6))
                            .child(t!("sync.choose_side").to_string()),
                    ),
            );
            return list;
        }

        if state.file_entries.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py(px(20.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.empty_dir").to_string()),
            );
            return list;
        }

        // Header row
        list = list.child(
            div()
                .flex()
                .items_center()
                .w_full()
                .px(px(8.0))
                .py(px(4.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().flex_grow().child(t!("sync.col.name").to_string()))
                .child(div().w(px(80.0)).flex_shrink_0().child(t!("sync.col.size").to_string()))
                .child(div().w(px(90.0)).flex_shrink_0().child(t!("sync.col.permissions").to_string()))
                .child(div().w(px(130.0)).flex_shrink_0().child(t!("sync.col.modified").to_string())),
        );

        // Parent directory ".." entry
        if state.current_path != "/" {
            let parent_path = {
                let p = std::path::Path::new(&state.current_path);
                p.parent()
                    .map(|pp| pp.to_string_lossy().to_string())
                    .unwrap_or_else(|| "/".to_string())
            };
            list = list.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "file-parent-{:?}",
                        side
                    ))))
                    .flex()
                    .items_center()
                    .w_full()
                    .px(px(8.0))
                    .py(px(3.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let state = match side {
                            PanelSide::Source => &mut this.source_panel,
                            PanelSide::Destination => &mut this.dest_panel,
                        };
                        if let Some(conn_id) = state.connection_id {
                            state.path_history.push(state.current_path.clone());
                            state.files_loading = true;
                            cx.emit(ServerSyncEvent::ListFiles {
                                connection_id: conn_id,
                                path: parent_path.clone(),
                                panel: side,
                            });
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .flex_grow()
                            .child(div().text_size(px(12.0)).child(".."))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(".."),
                            ),
                    ),
            );
        }

        for entry in &state.file_entries {
            let entry_path = entry.path.clone();
            let is_dir = entry.is_dir;
            let icon = if is_dir { "📁" } else { "📄" };
            let name = entry.name.clone();
            let size = entry.size_display();
            let perms = entry.permissions.clone();
            let modified = entry.modified.clone().unwrap_or_default();
            // Truncate modified to date portion
            let modified_short = modified.split('.').next().unwrap_or(&modified).to_string();

            list = list.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "file-{}-{:?}",
                        entry_path, side
                    ))))
                    .flex()
                    .items_center()
                    .w_full()
                    .px(px(8.0))
                    .py(px(3.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .when(is_dir, |el| {
                        el.on_click(cx.listener(move |this, _, _, cx| {
                            let state = match side {
                                PanelSide::Source => &mut this.source_panel,
                                PanelSide::Destination => &mut this.dest_panel,
                            };
                            if let Some(conn_id) = state.connection_id {
                                state.path_history.push(state.current_path.clone());
                                state.files_loading = true;
                                cx.emit(ServerSyncEvent::ListFiles {
                                    connection_id: conn_id,
                                    path: entry_path.clone(),
                                    panel: side,
                                });
                                cx.notify();
                            }
                        }))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .flex_grow()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(div().text_size(px(12.0)).child(icon.to_string()))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(if is_dir {
                                        ShellDeckColors::primary()
                                    } else {
                                        ShellDeckColors::text_primary()
                                    })
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(name),
                            ),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(size),
                    )
                    .child(
                        div()
                            .w(px(90.0))
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .font_family("JetBrains Mono")
                            .child(perms),
                    )
                    .child(
                        div()
                            .w(px(130.0))
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(modified_short),
                    ),
            );
        }

        list
    }

    // -----------------------------------------------------------------------
    // Discovery sections
    // -----------------------------------------------------------------------
    fn render_discovery_section(
        &self,
        side: PanelSide,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.panel_state(side);
        let has_connection = state.connection_id.is_some();
        let panel_height = state.discovery_panel_height;

        let mut section = div()
            .flex()
            .flex_col()
            .w_full()
            .flex_shrink_0()
            .border_t_1()
            .border_color(ShellDeckColors::border());

        if !has_connection {
            return section;
        }

        // Resize handle at top
        let handle_id = SharedString::from(format!("discovery-resize-{:?}", side));
        section = section.child(
            div()
                .id(ElementId::from(handle_id))
                .w_full()
                .h(px(4.0))
                .cursor_row_resize()
                .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.5)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        match side {
                            PanelSide::Source => this.source_panel.discovery_resizing = true,
                            PanelSide::Destination => this.dest_panel.discovery_resizing = true,
                        }
                        cx.notify();
                    }),
                ),
        );

        // Fixed header (SERVICES label + Discover button)
        section = section.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .flex_shrink_0()
                .px(px(8.0))
                .py(px(6.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("sync.services").to_string()),
                )
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "discover-btn-{:?}",
                            side
                        ))))
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(bg_tertiary())
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .cursor_pointer()
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let state = match side {
                                PanelSide::Source => &mut this.source_panel,
                                PanelSide::Destination => &mut this.dest_panel,
                            };
                            if let Some(conn_id) = state.connection_id {
                                state.discovery_loading = true;
                                cx.emit(ServerSyncEvent::DiscoverServices {
                                    connection_id: conn_id,
                                    panel: side,
                                });
                                cx.notify();
                            }
                        }))
                        .child(if state.discovery_loading {
                            t!("sync.discovering").to_string()
                        } else {
                            t!("sync.discover").to_string()
                        }),
                ),
        );

        // Scrollable content area with fixed height
        let scroll_id = SharedString::from(format!("discovery-scroll-{:?}", side));
        let mut content = div()
            .id(ElementId::from(scroll_id))
            .flex()
            .flex_col()
            .w_full()
            .h(px(panel_height))
            .overflow_y_scroll()
            .flex_shrink_0();

        // Nginx sites
        if !state.discovered_sites.is_empty() {
            content = content.child(
                div()
                    .px(px(8.0))
                    .py(px(2.0))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.nginx_sites").to_string()),
            );
            for site in &state.discovered_sites {
                let ssl_badge = if site.ssl {
                    t!("sync.ssl_badge").to_string()
                } else {
                    String::new()
                };
                let port_str = format!(":{}", site.listen_port);
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .py(px(3.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_primary())
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("{}{}", site.server_name, ssl_badge)),
                        )
                        .child(
                            div()
                                .px(px(4.0))
                                .py(px(1.0))
                                .rounded(px(3.0))
                                .bg(ShellDeckColors::badge_bg())
                                .text_size(px(9.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(port_str),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(site.root.clone()),
                        ),
                );
            }
        }

        // Databases
        if !state.discovered_databases.is_empty() {
            content = content.child(
                div()
                    .px(px(8.0))
                    .py(px(2.0))
                    .mt(px(4.0))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.databases").to_string()),
            );
            for db in &state.discovered_databases {
                let engine_label = db.engine.label();
                let size = db.size_display();
                let tables = db
                    .table_count
                    .map(|c| t!("sync.tables_count", count = c).to_string())
                    .unwrap_or_default();
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .py(px(3.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_primary())
                                .font_weight(FontWeight::MEDIUM)
                                .child(db.name.clone()),
                        )
                        .child(
                            div()
                                .px(px(4.0))
                                .py(px(1.0))
                                .rounded(px(3.0))
                                .bg(ShellDeckColors::primary().opacity(0.15))
                                .text_size(px(9.0))
                                .text_color(ShellDeckColors::primary())
                                .child(engine_label.to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(format!("{} · {}", size, tables)),
                        ),
                );
            }
        }

        // Empty state
        if state.discovered_sites.is_empty() && state.discovered_databases.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py(px(16.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.discover_hint").to_string()),
            );
        }

        section = section.child(content);
        section
    }

    // -----------------------------------------------------------------------
    // Server panel (one side)
    // -----------------------------------------------------------------------
    fn render_server_panel(&self, side: PanelSide, cx: &mut Context<Self>) -> impl IntoElement {
        let (label, accent_color) = match side {
            PanelSide::Source => (t!("sync.panel.source").to_string(), ShellDeckColors::success()),
            PanelSide::Destination => (
                t!("sync.panel.destination").to_string(),
                ShellDeckColors::primary(),
            ),
        };

        let state = self.panel_state(side);
        let status_text = if state.is_local {
            local_machine_label()
        } else if state.connection_name.is_empty() {
            t!("sync.not_connected").to_string()
        } else {
            state.connection_name.clone()
        };
        let status_line = t!("sync.panel_status", name = status_text.as_str()).to_string();

        let mut panel = div()
            .flex()
            .flex_col()
            .flex_grow()
            .min_w(px(200.0))
            .h_full()
            .overflow_hidden()
            .bg(ShellDeckColors::bg_primary())
            // Panel header with color accent
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(accent_color.opacity(0.3))
                    .bg(accent_color.opacity(0.12))
                    .child(
                        div()
                            .w(px(3.0))
                            .h(px(14.0))
                            .rounded(px(2.0))
                            .bg(accent_color)
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent_color)
                            .flex_shrink_0()
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_grow()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .truncate()
                            .child(status_line),
                    ),
            )
            // Connection picker — top padding separates it from the header band.
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .px(px(10.0))
                    .pt(px(10.0))
                    .pb(px(6.0))
                    .child(self.render_connection_picker(side, cx)),
            );

        // Breadcrumbs — only once a connection is picked. Otherwise the
        // panel would render a stray "/" root crumb under an empty dropdown.
        if self.panel_state(side).connection_id.is_some() {
            panel = panel.child(self.render_breadcrumbs(side, cx));
        }

        panel
            // File list
            .child(self.render_file_list(side, cx))
            // Discovery
            .child(self.render_discovery_section(side, cx))
    }

    // -----------------------------------------------------------------------
    // Panel divider
    // -----------------------------------------------------------------------
    fn render_panel_divider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sync-panel-divider")
            .w(px(4.0))
            .h_full()
            .bg(ShellDeckColors::border())
            .cursor_col_resize()
            .hover(|el| el.bg(ShellDeckColors::primary()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.panel_dragging = true;
                    cx.notify();
                }),
            )
    }

    // -----------------------------------------------------------------------
    // Log panel
    // -----------------------------------------------------------------------
    fn render_log_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_operation = self.active_operation.is_some();
        let has_logs = !self.log_lines.is_empty();

        if !has_operation && !has_logs {
            return div().id("log-panel-empty");
        }

        let mut panel = div()
            .id("log-panel")
            .flex()
            .flex_col()
            .w_full()
            .h(px(self.log_panel_height))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .bg(bg_secondary());

        // Resize handle
        panel = panel.child(
            div()
                .id("log-resize-handle")
                .w_full()
                .h(px(4.0))
                .cursor_row_resize()
                .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.5)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.log_panel_resizing = true;
                        cx.notify();
                    }),
                ),
        );

        // Header
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(10.0))
            .py(px(4.0));

        let mut header_left = div().flex().items_center().gap(px(6.0));
        header_left = header_left.child(
            div()
                .text_size(px(10.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_muted())
                .child(t!("sync.log_title").to_string()),
        );

        if has_operation {
            header_left = header_left.child(
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(ShellDeckColors::success()),
            );
        }

        header = header.child(header_left);

        // Log action buttons
        let mut log_actions = div().flex().items_center().gap(px(4.0));

        // Copy button
        let log_text = self.log_lines.join("\n");
        log_actions = log_actions.child(
            div()
                .id("copy-log-btn")
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .on_click(cx.listener(move |_this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(log_text.clone()));
                }))
                .child(t!("sync.copy").to_string()),
        );

        // Clear button
        log_actions = log_actions.child(
            div()
                .id("clear-log-btn")
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.log_lines.clear();
                    cx.notify();
                }))
                .child(t!("sync.clear").to_string()),
        );

        header = header.child(log_actions);

        panel = panel.child(header);

        // Progress bars (if operation active)
        if let Some(ref op) = self.active_operation {
            for prog in &op.item_progress {
                if prog.status == SyncOperationStatus::Running {
                    let pct = prog.percent().unwrap_or(0.0);
                    let label = prog.current_file.clone().unwrap_or_default();
                    panel = panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .px(px(10.0))
                            .py(px(2.0))
                            .gap(px(2.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(label),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(
                                                t!(
                                                    "sync.progress.percent",
                                                    pct = format!("{:.0}", pct).as_str()
                                                )
                                                .to_string(),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(px(3.0))
                                    .rounded(px(2.0))
                                    .bg(bg_tertiary())
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded(px(2.0))
                                            .bg(ShellDeckColors::primary())
                                            .w(relative(pct as f32 / 100.0)),
                                    ),
                            ),
                    );
                }
            }
        }

        // Log lines
        let mut log_scroll = div()
            .id("sync-log-scroll")
            .flex()
            .flex_col()
            .flex_grow()
            .overflow_y_scroll()
            .min_h(px(0.0))
            .px(px(10.0))
            .py(px(4.0));

        for line in self.log_lines.iter() {
            log_scroll = log_scroll.child(
                div()
                    .text_size(px(11.0))
                    .font_family("JetBrains Mono")
                    .text_color(ShellDeckColors::text_muted())
                    .whitespace_nowrap()
                    .child(line.clone()),
            );
        }

        panel = panel.child(log_scroll);
        panel
    }

    // -----------------------------------------------------------------------
    // Wizard modal
    // -----------------------------------------------------------------------
    fn render_wizard(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.wizard_active {
            return div().id("wizard-empty");
        }

        let step_title = match self.wizard_step {
            WizardStep::SelectItems => t!("sync.wizard.step.select_items").to_string(),
            WizardStep::ConfigureOptions => t!("sync.wizard.step.configure").to_string(),
            WizardStep::ReviewConfirm => t!("sync.wizard.step.review").to_string(),
            WizardStep::Executing => t!("sync.wizard.step.executing").to_string(),
        };

        // Backdrop
        div()
            .id("wizard-backdrop")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(hsla(0.0, 0.0, 0.0, 0.5))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                if this.wizard_step != WizardStep::Executing {
                    this.wizard_active = false;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id("wizard-modal")
                    .w(px(700.0))
                    .max_h(px(600.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(12.0))
                    .shadow_xl()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_click(cx.listener(|_, _, _, _| {
                        // Stop click propagation to backdrop
                    }))
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(20.0))
                            .py(px(14.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(step_title),
                            )
                            .child(self.render_wizard_step_indicator()),
                    )
                    // Body
                    .child(self.render_wizard_body(cx))
                    // Footer
                    .child(self.render_wizard_footer(cx)),
            )
    }

    fn render_wizard_step_indicator(&self) -> impl IntoElement {
        let steps = [
            t!("sync.wizard.indicator.items").to_string(),
            t!("sync.wizard.indicator.options").to_string(),
            t!("sync.wizard.indicator.review").to_string(),
            t!("sync.wizard.indicator.execute").to_string(),
        ];
        let current = match self.wizard_step {
            WizardStep::SelectItems => 0,
            WizardStep::ConfigureOptions => 1,
            WizardStep::ReviewConfirm => 2,
            WizardStep::Executing => 3,
        };

        let mut indicator = div().flex().items_center().gap(px(4.0));
        for (i, step_name) in steps.iter().enumerate() {
            let is_current = i == current;
            let is_done = i < current;
            indicator = indicator.child(
                div()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(10.0))
                    .text_size(px(10.0))
                    .when(is_current, |el| {
                        el.bg(ShellDeckColors::primary())
                            .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                    })
                    .when(is_done, |el| {
                        el.bg(ShellDeckColors::success().opacity(0.2))
                            .text_color(ShellDeckColors::success())
                    })
                    .when(!is_current && !is_done, |el| {
                        el.bg(bg_tertiary())
                            .text_color(ShellDeckColors::text_muted())
                    })
                    .child(step_name.clone()),
            );
        }
        indicator
    }

    fn render_wizard_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .id("wizard-body")
            .flex()
            .flex_col()
            .flex_grow()
            .overflow_y_scroll()
            .px(px(20.0))
            .py(px(16.0))
            .min_h(px(0.0));

        match self.wizard_step {
            WizardStep::SelectItems => {
                body = body.child(self.render_wizard_select_items(cx));
            }
            WizardStep::ConfigureOptions => {
                body = body.child(self.render_wizard_options(cx));
            }
            WizardStep::ReviewConfirm => {
                body = body.child(self.render_wizard_review(cx));
            }
            WizardStep::Executing => {
                body = body.child(self.render_wizard_executing(cx));
            }
        }

        body
    }

    fn render_wizard_select_items(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().flex().flex_col().gap(px(8.0));

        // Discovered sites from source
        if !self.source_panel.discovered_sites.is_empty() {
            content = content.child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.nginx_source").to_string()),
            );
            for (i, site) in self.source_panel.discovered_sites.iter().enumerate() {
                let site_name = site.server_name.clone();
                let root = site.root.clone();
                let is_selected = self.wizard_items.iter().any(|item| {
                    matches!(&item.kind, SyncItemKind::NginxSite { site: s, .. } if s.server_name == site_name)
                });
                let site_clone = site.clone();
                content = content.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!("wizard-site-{}", i))))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .border_1()
                        .when(is_selected, |el| {
                            el.border_color(ShellDeckColors::primary())
                                .bg(ShellDeckColors::primary().opacity(0.05))
                        })
                        .when(!is_selected, |el| {
                            el.border_color(ShellDeckColors::border())
                        })
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let site_name = site_clone.server_name.clone();
                            if let Some(idx) = this.wizard_items.iter().position(|item| {
                                matches!(&item.kind, SyncItemKind::NginxSite { site: s, .. } if s.server_name == site_name)
                            }) {
                                this.wizard_items.remove(idx);
                            } else {
                                this.wizard_items.push(SyncItem {
                                    id: Uuid::new_v4(),
                                    kind: SyncItemKind::NginxSite {
                                        site: site_clone.clone(),
                                        sync_config: true,
                                        sync_root: true,
                                    },
                                    enabled: true,
                                });
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .rounded(px(3.0))
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(is_selected, |el| {
                                    el.bg(ShellDeckColors::primary())
                                        .child(div().text_size(px(10.0)).text_color(hsla(0.0, 0.0, 1.0, 1.0)).child("✓"))
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div().text_size(px(12.0)).text_color(ShellDeckColors::text_primary()).child(site.server_name.clone()),
                                )
                                .child(
                                    div().text_size(px(10.0)).text_color(ShellDeckColors::text_muted()).child(root),
                                ),
                        ),
                );
            }
        }

        // Discovered databases from source
        if !self.source_panel.discovered_databases.is_empty() {
            content = content.child(
                div()
                    .mt(px(8.0))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.databases_source").to_string()),
            );
            for (i, db) in self.source_panel.discovered_databases.iter().enumerate() {
                let db_name = db.name.clone();
                let engine = db.engine;
                let is_selected = self.wizard_items.iter().any(|item| {
                    matches!(&item.kind, SyncItemKind::Database { name, .. } if *name == db_name)
                });
                content = content.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!("wizard-db-{}", i))))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .border_1()
                        .when(is_selected, |el| {
                            el.border_color(ShellDeckColors::primary())
                                .bg(ShellDeckColors::primary().opacity(0.05))
                        })
                        .when(!is_selected, |el| {
                            el.border_color(ShellDeckColors::border())
                        })
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let db_name_inner = db_name.clone();
                            if let Some(idx) = this.wizard_items.iter().position(|item| {
                                matches!(&item.kind, SyncItemKind::Database { name, .. } if *name == db_name_inner)
                            }) {
                                this.wizard_items.remove(idx);
                            } else {
                                this.wizard_items.push(SyncItem {
                                    id: Uuid::new_v4(),
                                    kind: SyncItemKind::Database {
                                        name: db_name_inner,
                                        engine,
                                        source_credentials: String::new(),
                                        dest_credentials: String::new(),
                                    },
                                    enabled: true,
                                });
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .rounded(px(3.0))
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(is_selected, |el| {
                                    el.bg(ShellDeckColors::primary())
                                        .child(div().text_size(px(10.0)).text_color(hsla(0.0, 0.0, 1.0, 1.0)).child("✓"))
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div().text_size(px(12.0)).text_color(ShellDeckColors::text_primary()).child(db.name.clone()),
                                )
                                .child(
                                    div()
                                        .px(px(4.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(ShellDeckColors::primary().opacity(0.15))
                                        .text_size(px(9.0))
                                        .text_color(ShellDeckColors::primary())
                                        .child(engine.label().to_string()),
                                )
                                .child(
                                    div().text_size(px(10.0)).text_color(ShellDeckColors::text_muted()).child(db.size_display()),
                                ),
                        ),
                );
            }
        }

        // Add directory button
        content = content.child(
            div()
                .mt(px(8.0))
                .id("add-dir-btn")
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .on_click(cx.listener(|this, _, _, cx| {
                    // Add current source path as a directory sync item
                    let source_path = this.source_panel.current_path.clone();
                    let dest_path = this.dest_panel.current_path.clone();
                    this.wizard_items.push(SyncItem {
                        id: Uuid::new_v4(),
                        kind: SyncItemKind::Directory {
                            source_path,
                            dest_path,
                            exclude_patterns: Vec::new(),
                        },
                        enabled: true,
                    });
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("sync.add_directory").to_string()),
                ),
        );

        // Show already added items
        if !self.wizard_items.is_empty() {
            content = content.child(
                div()
                    .mt(px(12.0))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(
                        t!("sync.items_selected", count = self.wizard_items.len()).to_string(),
                    ),
            );
        }

        content
    }

    fn render_wizard_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let opts = &self.wizard_options;

        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(self.render_option_toggle(
                t!("sync.option.compress").to_string(),
                opts.compress,
                cx,
                |this, val| {
                    this.wizard_options.compress = val;
                },
            ))
            .child(self.render_option_toggle(
                t!("sync.option.dry_run").to_string(),
                opts.dry_run,
                cx,
                |this, val| {
                    this.wizard_options.dry_run = val;
                },
            ))
            .child(self.render_option_toggle(
                t!("sync.option.delete_extra").to_string(),
                opts.delete_extra,
                cx,
                |this, val| {
                    this.wizard_options.delete_extra = val;
                },
            ))
            .child(self.render_option_toggle(
                t!("sync.option.skip_existing").to_string(),
                opts.skip_existing,
                cx,
                |this, val| {
                    this.wizard_options.skip_existing = val;
                },
            ))
            // Bandwidth limit
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("sync.bandwidth_limit").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(
                                opts.bandwidth_limit
                                    .map(|bw| t!("sync.bandwidth_value", kb = bw).to_string())
                                    .unwrap_or_else(|| t!("sync.bandwidth_unlimited").to_string()),
                            ),
                    ),
            )
    }

    fn render_option_toggle(
        &self,
        label: String,
        value: bool,
        cx: &mut Context<Self>,
        setter: fn(&mut Self, bool),
    ) -> impl IntoElement {
        let label_str = label;
        div()
            .id(ElementId::from(SharedString::from(format!(
                "opt-{}",
                label_str
            ))))
            .flex()
            .items_center()
            .justify_between()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                setter(this, !value);
                cx.notify();
            }))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(label_str),
            )
            .child(
                div()
                    .w(px(36.0))
                    .h(px(20.0))
                    .rounded(px(10.0))
                    .when(value, |el| el.bg(ShellDeckColors::primary()))
                    .when(!value, |el| el.bg(bg_tertiary()))
                    .child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded_full()
                            .bg(hsla(0.0, 0.0, 1.0, 1.0))
                            .mt(px(2.0))
                            .when(value, |el| el.ml(px(18.0)))
                            .when(!value, |el| el.ml(px(2.0))),
                    ),
            )
    }

    fn render_wizard_review(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().flex().flex_col().gap(px(8.0));

        content = content.child(
            div()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .font_weight(FontWeight::MEDIUM)
                .child(
                    t!(
                        "sync.review.summary",
                        count = self.wizard_items.len(),
                        source = self.source_panel.connection_name.as_str(),
                        dest = self.dest_panel.connection_name.as_str(),
                    )
                    .to_string(),
                ),
        );

        // Options summary
        let mut opts_summary = Vec::new();
        if self.wizard_options.compress {
            opts_summary.push(t!("sync.review.option.compress").to_string());
        }
        if self.wizard_options.dry_run {
            opts_summary.push(t!("sync.review.option.dry_run").to_string());
        }
        if self.wizard_options.delete_extra {
            opts_summary.push(t!("sync.review.option.delete_extra").to_string());
        }
        if self.wizard_options.skip_existing {
            opts_summary.push(t!("sync.review.option.skip_existing").to_string());
        }

        if !opts_summary.is_empty() {
            content = content.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(
                            "sync.review.options",
                            options = opts_summary.join(", ").as_str()
                        )
                        .to_string(),
                    ),
            );
        }

        // Item list
        for item in &self.wizard_items {
            let label = item.kind.label();
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(bg_secondary())
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(label),
                    ),
            );
        }

        content
    }

    fn render_wizard_executing(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div().flex().flex_col().gap(px(8.0));

        if let Some(ref op) = self.active_operation {
            // Overall progress
            let pct = op.overall_percent().unwrap_or(0.0);
            content = content
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(
                                    t!(
                                        "sync.progress.overall",
                                        pct = format!("{:.0}", pct).as_str()
                                    )
                                    .to_string(),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(format!("{:?}", op.status)),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.0))
                        .rounded(px(3.0))
                        .bg(bg_tertiary())
                        .child(
                            div()
                                .h_full()
                                .rounded(px(3.0))
                                .bg(ShellDeckColors::primary())
                                .w(relative(pct as f32 / 100.0)),
                        ),
                );

            // Per-item progress
            for prog in &op.item_progress {
                let item_pct = prog.percent().unwrap_or(0.0);
                let status_color = match prog.status {
                    SyncOperationStatus::Completed => ShellDeckColors::success(),
                    SyncOperationStatus::Failed => ShellDeckColors::error(),
                    SyncOperationStatus::Running => ShellDeckColors::primary(),
                    _ => ShellDeckColors::text_muted(),
                };
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(status_color))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(
                                    t!(
                                        "sync.progress.percent",
                                        pct = format!("{:.0}", item_pct).as_str()
                                    )
                                    .to_string(),
                                ),
                        )
                        .when_some(prog.current_file.clone(), |el, file| {
                            el.child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(file),
                            )
                        }),
                );
            }
        } else {
            content = content.child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("sync.preparing").to_string()),
            );
        }

        // Recent log lines
        let recent: Vec<_> = self.log_lines.iter().rev().take(10).collect();
        if !recent.is_empty() {
            content = content.child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .p(px(8.0))
                    .rounded(px(6.0))
                    .bg(bg_secondary())
                    .children(recent.into_iter().rev().map(|line| {
                        div()
                            .text_size(px(10.0))
                            .font_family("JetBrains Mono")
                            .text_color(ShellDeckColors::text_muted())
                            .child(line.clone())
                    })),
            );
        }

        content
    }

    fn render_wizard_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut footer = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(ShellDeckColors::border());

        // Cancel / Back
        let mut left = div().flex().items_center().gap(px(8.0));

        if self.wizard_step != WizardStep::Executing {
            left = left.child(
                div()
                    .id("wizard-cancel")
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.wizard_active = false;
                        cx.notify();
                    }))
                    .child(t!("sync.cancel").to_string()),
            );
        }

        if self.wizard_step == WizardStep::ConfigureOptions
            || self.wizard_step == WizardStep::ReviewConfirm
        {
            left = left.child(
                div()
                    .id("wizard-back")
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .cursor_pointer()
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.wizard_step = match this.wizard_step {
                            WizardStep::ConfigureOptions => WizardStep::SelectItems,
                            WizardStep::ReviewConfirm => WizardStep::ConfigureOptions,
                            _ => this.wizard_step,
                        };
                        cx.notify();
                    }))
                    .child(t!("sync.back").to_string()),
            );
        }

        footer = footer.child(left);

        // Next / Start / Cancel-Running
        let mut right = div();
        match self.wizard_step {
            WizardStep::SelectItems => {
                let has_items = !self.wizard_items.is_empty();
                right = right.child(
                    div()
                        .id("wizard-next-1")
                        .px(px(14.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .when(has_items, |el| {
                            el.bg(ShellDeckColors::primary())
                                .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                                .hover(|el| el.opacity(0.9))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.wizard_step = WizardStep::ConfigureOptions;
                                    cx.notify();
                                }))
                        })
                        .when(!has_items, |el| {
                            el.bg(bg_tertiary())
                                .text_color(ShellDeckColors::text_muted())
                        })
                        .child(t!("sync.next").to_string()),
                );
            }
            WizardStep::ConfigureOptions => {
                right = right.child(
                    div()
                        .id("wizard-next-2")
                        .px(px(14.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::primary())
                        .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|el| el.opacity(0.9))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.wizard_step = WizardStep::ReviewConfirm;
                            cx.notify();
                        }))
                        .child(t!("sync.next").to_string()),
                );
            }
            WizardStep::ReviewConfirm => {
                right = right.child(
                    div()
                        .id("wizard-start")
                        .px(px(14.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::success())
                        .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|el| el.opacity(0.9))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.wizard_step = WizardStep::Executing;
                            // Build and emit the sync profile
                            if let (Some(src_id), Some(dest_id)) = (
                                this.source_panel.connection_id,
                                this.dest_panel.connection_id,
                            ) {
                                let profile = SyncProfile {
                                    id: Uuid::new_v4(),
                                    name: format!(
                                        "{} -> {}",
                                        this.source_panel.connection_name,
                                        this.dest_panel.connection_name
                                    ),
                                    description: None,
                                    source_connection_id: src_id,
                                    dest_connection_id: dest_id,
                                    items: this.wizard_items.clone(),
                                    options: this.wizard_options.clone(),
                                    created_at: chrono::Utc::now(),
                                    last_synced: None,
                                };
                                cx.emit(ServerSyncEvent::StartSync(profile));
                            }
                            cx.notify();
                        }))
                        .child(t!("sync.start").to_string()),
                );
            }
            WizardStep::Executing => {
                right = right.child(
                    div()
                        .id("wizard-cancel-exec")
                        .px(px(14.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::error())
                        .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|el| el.opacity(0.9))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(ref op) = this.active_operation {
                                cx.emit(ServerSyncEvent::CancelSync(op.id));
                            }
                            cx.notify();
                        }))
                        .child(t!("sync.cancel_sync").to_string()),
                );
            }
        }

        footer = footer.child(right);
        footer
    }
}

impl Render for ServerSyncView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    if this.wizard_active && this.wizard_step != WizardStep::Executing {
                        this.wizard_active = false;
                        cx.notify();
                    }
                }
            }));

        // Toolbar
        root = root.child(self.render_toolbar(cx));

        // Main panels area
        let total_w = 1.0_f32; // relative

        let mut panels = div().flex().flex_grow().min_h(px(0.0)).overflow_hidden();

        panels = panels
            .child(
                div()
                    .flex_basis(relative(self.panel_ratio))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_server_panel(PanelSide::Source, cx)),
            )
            .child(self.render_panel_divider(cx))
            .child(
                div()
                    .flex_basis(relative(total_w - self.panel_ratio))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_server_panel(PanelSide::Destination, cx)),
            );

        root = root.child(panels);

        // Log panel
        root = root.child(self.render_log_panel(cx));

        // Wizard overlay
        root = root.child(self.render_wizard(cx));

        root
    }
}
