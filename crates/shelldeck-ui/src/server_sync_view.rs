use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::connection::Connection;
use shelldeck_core::models::server_sync::*;
use uuid::Uuid;

use crate::theme::ShellDeckColors;

// Theme helpers — map semantic names to existing ShellDeckColors methods.
fn bg_secondary() -> gpui::Hsla {
    ShellDeckColors::bg_surface()
}
fn bg_tertiary() -> gpui::Hsla {
    ShellDeckColors::bg_sidebar()
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
    source_picker_open: bool,
    dest_picker_open: bool,
    pub log_panel_height: f32,
    pub log_panel_resizing: bool,
    focus_handle: FocusHandle,
    pub panel_ratio: f32,
    pub panel_dragging: bool,
}

impl EventEmitter<ServerSyncEvent> for ServerSyncView {}

impl ServerSyncView {
    pub fn new(cx: &mut Context<Self>) -> Self {
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
            source_picker_open: false,
            dest_picker_open: false,
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

    pub fn set_connections(&mut self, connections: Vec<Connection>) {
        self.connections = connections;
    }

    pub fn set_profiles(&mut self, profiles: Vec<SyncProfile>) {
        self.profiles = profiles;
    }

    pub fn load_profile(&mut self, profile_id: Uuid) {
        self.selected_profile = Some(profile_id);
        if let Some(profile) = self.profiles.iter().find(|p| p.id == profile_id) {
            // Set source connection
            if profile.source_connection_id == LOCAL_MACHINE_ID {
                self.source_panel.connection_id = Some(LOCAL_MACHINE_ID);
                self.source_panel.connection_name = "Local Machine".to_string();
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
                self.dest_panel.connection_name = "Local Machine".to_string();
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
        let mut left = div().flex().items_center().gap(px(12.0));
        left = left.child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_primary())
                .child("Server Sync"),
        );

        // Profile dropdown
        if !self.profiles.is_empty() {
            let mut profile_section = div().flex().items_center().gap(px(4.0));
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
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .border_1()
                        .text_size(px(11.0))
                        .cursor_pointer()
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
                            this.load_profile(pid);
                            cx.notify();
                        }))
                        .child(pname),
                );
            }
            left = left.child(profile_section);
        }

        toolbar = toolbar.child(left);

        // Right side: buttons
        let mut right = div().flex().items_center().gap(px(8.0));

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
                    .child("Save Profile"),
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
                    .child("Delete"),
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
                .child("Start Sync"),
        );

        toolbar = toolbar.child(right);
        toolbar
    }

    // -----------------------------------------------------------------------
    // Connection picker
    // -----------------------------------------------------------------------
    fn render_connection_picker(
        &self,
        side: PanelSide,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.panel_state(side);
        let label = if state.connection_name.is_empty() {
            match side {
                PanelSide::Source => "Select Source...",
                PanelSide::Destination => "Select Destination...",
            }
        } else {
            &state.connection_name
        };
        let is_local = state.is_local;

        let picker_open = match side {
            PanelSide::Source => self.source_picker_open,
            PanelSide::Destination => self.dest_picker_open,
        };

        let mut picker = div().flex().flex_col().w_full().relative();

        // Picker button
        picker = picker.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "conn-picker-{:?}",
                    side
                ))))
                .flex()
                .items_center()
                .justify_between()
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .cursor_pointer()
                .hover(|el| el.border_color(ShellDeckColors::primary()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    match side {
                        PanelSide::Source => this.source_picker_open = !this.source_picker_open,
                        PanelSide::Destination => this.dest_picker_open = !this.dest_picker_open,
                    }
                    cx.notify();
                }))
                .when(state.connection_id.is_some(), |el| {
                    let indicator_color = if is_local {
                        ShellDeckColors::success()
                    } else {
                        ShellDeckColors::primary()
                    };
                    el.child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded_full()
                            .bg(indicator_color),
                    )
                })
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(if state.connection_id.is_some() {
                            ShellDeckColors::text_primary()
                        } else {
                            ShellDeckColors::text_muted()
                        })
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(if picker_open { "▲" } else { "▼" }),
                ),
        );

        // Dropdown list
        if picker_open {
            let mut dropdown = div()
                .id(ElementId::from(SharedString::from(format!(
                    "conn-dropdown-{:?}",
                    side
                ))))
                .absolute()
                .top(px(40.0))
                .left_0()
                .w_full()
                .max_h(px(200.0))
                .overflow_y_scroll()
                .bg(bg_secondary())
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(6.0))
                .shadow_md();

            // Local Machine option (always first)
            let is_local_selected = state.is_local;
            dropdown = dropdown.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "conn-opt-local-{:?}",
                        side
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .cursor_pointer()
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .when(is_local_selected, |el| {
                        el.bg(ShellDeckColors::primary().opacity(0.08))
                    })
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
                        let state = match side {
                            PanelSide::Source => &mut this.source_panel,
                            PanelSide::Destination => &mut this.dest_panel,
                        };
                        state.connection_id = Some(LOCAL_MACHINE_ID);
                        state.connection_name = "Local Machine".to_string();
                        state.is_local = true;
                        state.current_path = home.clone();
                        state.file_entries.clear();
                        state.discovered_sites.clear();
                        state.discovered_databases.clear();
                        state.files_loading = true;

                        match side {
                            PanelSide::Source => this.source_picker_open = false,
                            PanelSide::Destination => this.dest_picker_open = false,
                        }

                        cx.emit(ServerSyncEvent::ListFiles {
                            connection_id: LOCAL_MACHINE_ID,
                            path: home,
                            panel: side,
                        });
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded(px(4.0))
                            .bg(ShellDeckColors::success().opacity(0.15))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::success())
                            .child("~"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_primary())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Local Machine"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child("This computer"),
                            ),
                    ),
            );

            // Remote connections
            for conn in &self.connections {
                let conn_id = conn.id;
                let conn_name = conn.display_name().to_string();
                let hostname = conn.hostname.clone();
                let display = conn_name.clone();
                dropdown = dropdown.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "conn-opt-{}-{:?}",
                            conn_id, side
                        ))))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .w_full()
                        .px(px(10.0))
                        .py(px(6.0))
                        .cursor_pointer()
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let state = match side {
                                PanelSide::Source => &mut this.source_panel,
                                PanelSide::Destination => &mut this.dest_panel,
                            };
                            state.connection_id = Some(conn_id);
                            state.connection_name = display.clone();
                            state.is_local = false;
                            state.current_path = "/".to_string();
                            state.file_entries.clear();
                            state.discovered_sites.clear();
                            state.discovered_databases.clear();
                            state.files_loading = true;

                            match side {
                                PanelSide::Source => this.source_picker_open = false,
                                PanelSide::Destination => this.dest_picker_open = false,
                            }

                            cx.emit(ServerSyncEvent::ListFiles {
                                connection_id: conn_id,
                                path: "/".to_string(),
                                panel: side,
                            });
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(24.0))
                                .h(px(24.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::primary().opacity(0.15))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::primary())
                                .child("@"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(conn_name),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(hostname),
                                ),
                        ),
                );
            }

            picker = picker.child(dropdown);
        }

        picker
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
            .px(px(8.0))
            .py(px(4.0))
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
                    .child("Loading..."),
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
                            .child("Select a connection above"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted().opacity(0.6))
                            .child("Choose Local Machine or a remote server"),
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
                    .child("Empty directory"),
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
                .child(div().flex_grow().child("Name"))
                .child(div().w(px(80.0)).flex_shrink_0().child("Size"))
                .child(div().w(px(90.0)).flex_shrink_0().child("Permissions"))
                .child(div().w(px(130.0)).flex_shrink_0().child("Modified")),
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
                        .child("SERVICES"),
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
                            "Discovering..."
                        } else {
                            "Discover"
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
                    .child("Nginx Sites"),
            );
            for site in &state.discovered_sites {
                let ssl_badge = if site.ssl { " [SSL]" } else { "" };
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
                    .child("Databases"),
            );
            for db in &state.discovered_databases {
                let engine_label = db.engine.label();
                let size = db.size_display();
                let tables = db
                    .table_count
                    .map(|c| format!("{} tables", c))
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
                    .child("Click Discover to scan services"),
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
            PanelSide::Source => ("SOURCE", ShellDeckColors::success()),
            PanelSide::Destination => ("DESTINATION", ShellDeckColors::primary()),
        };

        let state = self.panel_state(side);
        let status_text = if state.is_local {
            "Local Machine"
        } else if state.connection_name.is_empty() {
            "Not connected"
        } else {
            &state.connection_name
        };

        div()
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
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(accent_color.opacity(0.3))
                    .bg(accent_color.opacity(0.05))
                    .child(
                        div()
                            .w(px(3.0))
                            .h(px(14.0))
                            .rounded(px(2.0))
                            .bg(accent_color),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(accent_color)
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("— {}", status_text)),
                    ),
            )
            // Connection picker
            .child(
                div()
                    .px(px(8.0))
                    .pb(px(4.0))
                    .child(self.render_connection_picker(side, cx)),
            )
            // Breadcrumbs
            .child(self.render_breadcrumbs(side, cx))
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
                .child("SYNC LOG"),
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
                .child("Copy"),
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
                .child("Clear"),
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
                                            .child(format!("{:.0}%", pct)),
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
            WizardStep::SelectItems => "Select Items to Sync",
            WizardStep::ConfigureOptions => "Configure Options",
            WizardStep::ReviewConfirm => "Review & Confirm",
            WizardStep::Executing => "Syncing...",
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
        let steps = ["Items", "Options", "Review", "Execute"];
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
                    .child(step_name.to_string()),
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
                    .child("Nginx Sites (Source)"),
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
                    .child("Databases (Source)"),
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
                        .child("+ Add Directory (current paths)"),
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
                    .child(format!("{} items selected", self.wizard_items.len())),
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
                "Compress transfers",
                opts.compress,
                cx,
                |this, val| {
                    this.wizard_options.compress = val;
                },
            ))
            .child(self.render_option_toggle(
                "Dry run (preview only)",
                opts.dry_run,
                cx,
                |this, val| {
                    this.wizard_options.dry_run = val;
                },
            ))
            .child(self.render_option_toggle(
                "Delete extra files on destination",
                opts.delete_extra,
                cx,
                |this, val| {
                    this.wizard_options.delete_extra = val;
                },
            ))
            .child(self.render_option_toggle(
                "Skip existing files",
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
                            .child("Bandwidth limit (KB/s)"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(
                                opts.bandwidth_limit
                                    .map(|bw| format!("{} KB/s", bw))
                                    .unwrap_or_else(|| "Unlimited".to_string()),
                            ),
                    ),
            )
    }

    fn render_option_toggle(
        &self,
        label: &str,
        value: bool,
        cx: &mut Context<Self>,
        setter: fn(&mut Self, bool),
    ) -> impl IntoElement {
        let label_str = label.to_string();
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
                .child(format!(
                    "Sync {} items from {} to {}",
                    self.wizard_items.len(),
                    self.source_panel.connection_name,
                    self.dest_panel.connection_name,
                )),
        );

        // Options summary
        let mut opts_summary = Vec::new();
        if self.wizard_options.compress {
            opts_summary.push("compress");
        }
        if self.wizard_options.dry_run {
            opts_summary.push("dry run");
        }
        if self.wizard_options.delete_extra {
            opts_summary.push("delete extra");
        }
        if self.wizard_options.skip_existing {
            opts_summary.push("skip existing");
        }

        if !opts_summary.is_empty() {
            content = content.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("Options: {}", opts_summary.join(", "))),
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
                                .child(format!("Overall: {:.0}%", pct)),
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
                                .child(format!("{:.0}%", item_pct)),
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
                    .child("Preparing sync..."),
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
                    .child("Cancel"),
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
                    .child("Back"),
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
                        .child("Next"),
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
                        .child("Next"),
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
                        .child("Start Sync"),
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
                        .child("Cancel Sync"),
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
                    } else if this.source_picker_open {
                        this.source_picker_open = false;
                        cx.notify();
                    } else if this.dest_picker_open {
                        this.dest_picker_open = false;
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
