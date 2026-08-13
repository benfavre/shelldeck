use super::*;

impl Workspace {
    // --- Manage sites (site switcher) ---

    pub(super) fn issue_site_option_label(site: &ManagedSiteInfo) -> String {
        let label = site.display_label();
        if site.host.trim().is_empty() || label.contains(site.host.trim()) {
            label
        } else {
            format!("{label} — {}", site.host.trim())
        }
    }

    pub(super) fn build_issue_site_select(
        sites: &[ManagedSiteInfo],
        selected_site_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Entity<Select<String>> {
        let mut sorted_sites = sites.to_vec();
        sorted_sites.sort_by_key(|site| Self::issue_site_option_label(site).to_lowercase());

        let mut options =
            vec![
                SelectOption::new(String::new(), t!("user.requests.site_none").to_string())
                    .with_icon("icons/lucide/globe.svg"),
            ];
        options.extend(sorted_sites.into_iter().map(|site| {
            let mut option =
                SelectOption::new(site.site_id.clone(), Self::issue_site_option_label(&site))
                    .with_icon("icons/lucide/globe.svg");
            if !site.tenant_name.trim().is_empty() {
                option = option.with_group(site.tenant_name.clone());
            }
            option
        }));

        let selected_index = selected_site_id
            .and_then(|id| options.iter().position(|option| option.value == id))
            .or(Some(0));
        let parent = cx.entity();
        cx.new(move |select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .placeholder(t!("user.requests.site_placeholder").to_string())
                .searchable(true)
                .search_placeholder(t!("user.requests.site_placeholder").to_string())
                .leading_icon("icons/lucide/search.svg")
                .on_change(move |site_id, _window, cx| {
                    parent.update(cx, |workspace, cx| {
                        workspace.issue_new_site_id = if site_id.is_empty() {
                            None
                        } else {
                            Some(site_id.clone())
                        };
                        cx.notify();
                    });
                })
        })
    }

    pub(super) fn rebuild_issue_site_select(&mut self, cx: &mut Context<Self>) {
        let sites = self
            .site_directory
            .as_ref()
            .map(|directory| directory.sites.as_slice())
            .unwrap_or_default();
        if self
            .issue_new_site_id
            .as_ref()
            .is_some_and(|selected| !sites.iter().any(|site| &site.site_id == selected))
        {
            self.issue_new_site_id = None;
        }
        let selected = self.issue_new_site_id.as_deref();
        self.issue_site_select = Self::build_issue_site_select(sites, selected, cx);
    }

    pub(super) fn reset_new_request_site_to_active(&mut self, cx: &mut Context<Self>) {
        self.issue_new_site_id = self
            .app_config
            .cloud_sync
            .active_site_id
            .as_ref()
            .filter(|active_id| {
                self.site_directory.as_ref().is_some_and(|directory| {
                    directory.sites.iter().any(|s| &s.site_id == *active_id)
                })
            })
            .cloned();
        self.rebuild_issue_site_select(cx);
    }

    /// Fetch the sites directory + areas in the background and cache them.
    /// No-op when logged out; never blocks.
    pub fn refresh_sites(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let base = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { manage_sites::fetch_sites(&base, &token) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(payload) => {
                    tracing::info!(
                        "Loaded {} manage sites, {} areas",
                        payload.sites.len(),
                        payload.areas.len()
                    );
                    ws.site_directory = Some(payload);
                    ws.rebuild_issue_site_select(cx);
                    ws.refresh_command_palette(cx);
                    // Server may have just delivered the Jean config (super-admin).
                    ws.update_jean_availability(cx);
                    ws.sync_jean_poll(cx);
                    cx.notify();
                }
                Err(e) => tracing::warn!("Failed to load manage sites: {}", e),
            });
        })
        .detach();
    }

    /// The `ManagedSiteInfo` for the persisted active site, if it's in the cache.
    pub(super) fn active_site_info(&self) -> Option<ManagedSiteInfo> {
        let id = self.app_config.cloud_sync.active_site_id.as_deref()?;
        self.site_directory
            .as_ref()?
            .sites
            .iter()
            .find(|s| s.site_id == id)
            .cloned()
    }

    /// Select the active site (or `None` for "all sites"): persist it, scope the
    /// sidebar, and close the dropdown.
    pub(super) fn select_site(
        &mut self,
        site_id: Option<String>,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let activity_site_id = site_id.clone();
        let activity_label = label.clone();
        self.app_config.cloud_sync.active_site_id = site_id.clone();
        self.app_config.cloud_sync.active_site_label = label;
        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to save active site: {}", e);
        }
        let filter = site_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
        self.sidebar.update(cx, |s, cx| {
            s.set_site_filter(filter);
            cx.notify();
        });
        self.refresh_command_palette(cx);
        self.site_menu_open = false;
        if let Some(site_id) = activity_site_id {
            let label = activity_label.unwrap_or_else(|| site_id.clone());
            self.add_activity_entry(
                ActivityEntry::new(
                    ActivityKind::Site,
                    t!("activity.site.selected", label = label.as_str()).to_string(),
                )
                .with_target(site_id, label)
                .with_action(ActivityAction::OpenSite),
                cx,
            );
        }
        cx.notify();
    }

    /// Open a manage area for the active site in the system browser.
    pub fn open_manage_area(&mut self, area_path: String, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let site = match self.active_site_info() {
            Some(s) => s,
            None => {
                self.show_toast(
                    t!("toast.select_active_site_first").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
                return;
            }
        };
        let origin = self
            .site_directory
            .as_ref()
            .map(|p| p.manage_origin.clone())
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| self.account_base_url());
        let url = manage_sites::manage_area_url(&origin, &site, &area_path);
        self.site_menu_open = false;
        match cloud_account::open_in_browser(&url) {
            Ok(_) => self.show_toast(
                t!("toast.opening_browser").to_string(),
                ToastLevel::Info,
                cx,
            ),
            Err(e) => self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = cloud_account::user_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
        cx.notify();
    }

    /// Open the titlebar site switcher (from the command palette / an action).
    pub fn open_site_switcher(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        if self.site_directory.is_none() {
            self.show_toast(
                t!("toast.login_required_site_switch").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        self.site_menu_open = true;
        self.theme_menu_open = false;
        self.account_menu_open = false;
        cx.notify();
    }
}
