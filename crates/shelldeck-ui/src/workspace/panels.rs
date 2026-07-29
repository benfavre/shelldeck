use super::*;

impl Workspace {
    // --- Contextual sidebar panel ---

    /// Push each rail activity's rows into the sidebar.
    ///
    /// Rebuilt per render like the menu bar: the sources are live entities
    /// (terminal tabs, scripts, forwards, editor buffers) with no single
    /// change signal between them, and every row is a small owned summary
    /// rather than a clone of the underlying model.
    pub(super) fn refresh_sidebar_panels(&mut self, cx: &mut Context<Self>) {
        use crate::sidebar::PanelItem;
        use shelldeck_core::models::port_forward::ForwardStatus;
        use shelldeck_terminal::session::SessionState;

        let terminals: Vec<PanelItem> = self
            .terminal
            .read(cx)
            .tabs
            .iter()
            .map(|tab| PanelItem {
                id: tab.id,
                label: tab.title.clone(),
                detail: tab.connection_id.and_then(|id| {
                    self.connections
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.display_name().to_string())
                }),
                icon: "terminal",
                is_active: tab.is_active,
                is_live: matches!(tab.state, SessionState::Running),
            })
            .collect();

        let selected_script = self.scripts.read(cx).selected_script;
        let scripts: Vec<PanelItem> = self
            .scripts
            .read(cx)
            .scripts
            .iter()
            .map(|script| PanelItem {
                id: script.id,
                label: script.name.clone(),
                detail: script.description.clone(),
                icon: "scroll-text",
                is_active: selected_script == Some(script.id),
                is_live: false,
            })
            .collect();

        let forwards: Vec<PanelItem> = self
            .port_forwards
            .read(cx)
            .forwards
            .iter()
            .map(|fwd| PanelItem {
                id: fwd.id,
                label: fwd
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("{} → {}", fwd.local_port, fwd.remote_port)),
                detail: Some(format!(
                    "{}:{} → {}:{}",
                    fwd.local_host, fwd.local_port, fwd.remote_host, fwd.remote_port
                )),
                icon: "arrow-left-right",
                is_active: false,
                is_live: matches!(fwd.status, ForwardStatus::Active),
            })
            .collect();

        // Manage keys the active site by its raw string id, so the comparison
        // happens on that rather than on the row's synthesized UUID.
        let active_site = self.app_config.cloud_sync.active_site_id.clone();
        let sites: Vec<PanelItem> = self
            .site_directory
            .as_ref()
            .map(|payload| {
                payload
                    .sites
                    .iter()
                    .map(|site| {
                        // Site ids are strings server-side; fall back to a
                        // stable name-derived id so rows keep distinct
                        // ElementIds even when the id is absent.
                        let id = Uuid::parse_str(&site.site_id)
                            .unwrap_or_else(|_| uuid_from_key(&site.site_id, &site.name));
                        PanelItem {
                            id,
                            label: if site.name.is_empty() {
                                site.label.clone()
                            } else {
                                site.name.clone()
                            },
                            detail: (!site.host.is_empty()).then(|| site.host.clone()),
                            icon: "globe",
                            is_active: active_site.as_deref() == Some(site.site_id.as_str()),
                            is_live: false,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let recent: Vec<PanelItem> = self
            .recent_activity
            .iter()
            .take(40)
            .map(|entry| PanelItem {
                id: entry.id,
                label: entry.message.clone(),
                detail: Some(rel_time(entry.at.timestamp_millis() as f64)),
                icon: "activity",
                is_active: false,
                is_live: false,
            })
            .collect();

        let active_editor_tab = self.file_editor.read(cx).active_tab_index;
        let editor_files: Vec<PanelItem> = self
            .file_editor
            .read(cx)
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| PanelItem {
                id: tab.id,
                label: tab.filename.clone(),
                detail: tab.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                icon: "pencil",
                is_active: index == active_editor_tab,
                is_live: false,
            })
            .collect();

        self.sidebar.update(cx, |sidebar, _| {
            sidebar.set_panel_items(SidebarSection::Terminals, terminals);
            sidebar.set_panel_items(SidebarSection::Scripts, scripts);
            sidebar.set_panel_items(SidebarSection::PortForwards, forwards);
            sidebar.set_panel_items(SidebarSection::Sites, sites);
            sidebar.set_panel_items(SidebarSection::Recent, recent);
            sidebar.set_panel_items(SidebarSection::FileEditor, editor_files);
        });
    }

    /// Route a click on a contextual panel row to the right action for its
    /// activity. Each arm reuses the existing entry point rather than
    /// duplicating selection logic.
    pub(super) fn handle_panel_item_selected(
        &mut self,
        section: SidebarSection,
        id: Uuid,
        cx: &mut Context<Self>,
    ) {
        match section {
            SidebarSection::Terminals => {
                self.active_view = ActiveView::Terminal;
                self.terminal.update(cx, |terminal, cx| {
                    terminal.select_tab(id);
                    cx.notify();
                });
            }
            SidebarSection::Scripts => {
                self.active_view = ActiveView::Scripts;
                self.scripts.update(cx, |editor, cx| {
                    editor.selected_script = Some(id);
                    cx.notify();
                });
            }
            SidebarSection::PortForwards => {
                self.active_view = ActiveView::PortForwards;
            }
            SidebarSection::Sites => {
                self.active_view = ActiveView::Sites;
            }
            SidebarSection::Recent => {
                self.active_view = ActiveView::Recent;
            }
            SidebarSection::FileEditor => {
                self.active_view = ActiveView::FileEditor;
                self.file_editor.update(cx, |editor, cx| {
                    if let Some(index) = editor.tabs.iter().position(|tab| tab.id == id) {
                        editor.active_tab_index = index;
                    }
                    cx.notify();
                });
            }
            _ => {}
        }
        cx.notify();
    }
}
