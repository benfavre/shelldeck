use super::*;

impl Workspace {
    pub(super) fn render_site_section_header(label: &str) -> impl IntoElement {
        div()
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(4.0))
            .text_size(px(10.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .child(label.to_string())
    }

    /// Open a manage area in the browser for a specific site (User-mode rows).
    pub(super) fn open_area_for_site(
        &mut self,
        site: ManagedSiteInfo,
        area_path: String,
        cx: &mut Context<Self>,
    ) {
        let origin = self
            .site_directory
            .as_ref()
            .map(|p| p.manage_origin.clone())
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| self.account_base_url());
        let url = manage_sites::manage_area_url(&origin, &site, &area_path);
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
    }

    /// Split the site directory into `(active, others)` — the active site
    /// as a full "rich" card, everyone else as compact virtualised rows.
    /// Applies the live search query, then sorts (active pinned, then
    /// connection-bearing, then alpha) so the compact list has a stable
    /// order. The active site is only returned when it *also* passes the
    /// filter — a filter that hides the current active means the top card
    /// disappears (the sidebar filter itself stays untouched).
    pub(super) fn partition_user_sites(
        &self,
        cx: &mut Context<Self>,
    ) -> (
        Option<manage_sites::ManagedSiteInfo>,
        Vec<manage_sites::ManagedSiteInfo>,
    ) {
        let payload = self.site_directory.clone().unwrap_or_default();
        let active_id = self.app_config.cloud_sync.active_site_id.clone();
        let conn_site_ids: std::collections::HashSet<String> = self
            .connections
            .iter()
            .filter_map(|c| c.site_id.map(|id| id.to_string()))
            .collect();
        let q = self
            .user_sites_search_state
            .read(cx)
            .content()
            .trim()
            .to_lowercase();
        let mut sites: Vec<manage_sites::ManagedSiteInfo> = payload
            .sites
            .iter()
            .filter(|s| {
                q.is_empty()
                    || s.display_label().to_lowercase().contains(&q)
                    || s.host.to_lowercase().contains(&q)
                    || s.tenant_name.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        sites.sort_by(|a, b| {
            let a_conn = conn_site_ids.contains(&a.site_id);
            let b_conn = conn_site_ids.contains(&b.site_id);
            b_conn.cmp(&a_conn).then(
                a.display_label()
                    .to_lowercase()
                    .cmp(&b.display_label().to_lowercase()),
            )
        });
        let active = active_id
            .as_deref()
            .and_then(|id| sites.iter().position(|s| s.site_id == id))
            .map(|idx| sites.remove(idx));
        (active, sites)
    }

    /// Full "rich" site card — reserved for the currently-active site. This
    /// is the only place areas + wp-admin chip render (the compact rows keep
    /// paint budget low by omitting them). Extracted from the pre-virt loop
    /// verbatim; only the `is_active = true` branch stays here (the compact
    /// row handles inactive sites now).
    pub(super) fn render_active_site_card(
        &self,
        site: &manage_sites::ManagedSiteInfo,
        area_buttons: &[manage_sites::ManageArea],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sid = site.site_id.clone();
        let label = site.display_label();
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::primary())
            .bg(ShellDeckColors::bg_sidebar());

        // Row 1: identity + "Site actif" pill.
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child({
                    let mut identity = div().flex().flex_col().min_w(px(0.0)).overflow_hidden();
                    let mut label_row = div().flex().items_center().gap(px(6.0)).child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .truncate()
                            .child(label.clone()),
                    );
                    if site.is_wordpress == Some(true) {
                        label_row = label_row.child(
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::primary())
                                .flex_shrink_0()
                                .child("WP"),
                        );
                    }
                    identity = identity.child(label_row).child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if site.host.is_empty() {
                                site.tenant_name.clone()
                            } else {
                                site.host.clone()
                            }),
                    );
                    identity
                })
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .flex_shrink_0()
                        .bg(ShellDeckColors::primary().opacity(0.15))
                        .text_color(ShellDeckColors::primary())
                        .child(t!("user.sites.active").to_string()),
                ),
        );

        // Row 2: wp-admin shortcut (if any) + area deep-links.
        let mut areas_row = div().flex().flex_wrap().gap(px(6.0));
        if let Some(wp_url) = site.wp_admin_url.as_ref().filter(|u| !u.is_empty()) {
            let wp_url_owned = wp_url.clone();
            areas_row = areas_row.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "uh-wp-{}",
                        sid
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(ShellDeckColors::primary().opacity(0.35))
                    .bg(ShellDeckColors::primary().opacity(0.08))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::primary())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::primary().opacity(0.14)))
                    .child(lucide_icon(
                        "external-link",
                        11.0,
                        ShellDeckColors::primary(),
                    ))
                    .child("wp-admin")
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, _cx| {
                        let _ =
                            shelldeck_core::config::cloud_account::open_in_browser(&wp_url_owned);
                    })),
            );
        }
        for area in area_buttons {
            let site_clone = site.clone();
            let path = area.path.clone();
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "uh-area-{}-{}",
                    sid, area.key
                ))))
                .flex()
                .items_center()
                .gap(px(5.0))
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|s| {
                    s.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                });
            if let Some(slug) = manage_area_icon(&area.key) {
                chip = chip.child(
                    svg()
                        .path(lucide_path(slug))
                        .size(px(11.0))
                        .text_color(ShellDeckColors::text_muted()),
                );
            }
            areas_row = areas_row.child(chip.child(area.label.clone()).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.open_area_for_site(site_clone.clone(), path.clone(), cx);
                },
            )));
        }
        card.child(areas_row)
    }

    /// Fixed-height compact row for a non-active site. The full slot
    /// (`SITE_ROW_H = 64px`) contains an inner card that's ~56px tall with
    /// 4px padding top/bottom, giving an 8px visual gap between adjacent
    /// rows without breaking `uniform_list`'s uniform-height contract.
    /// Width fills the parent (`w_full`) so rows land on the same right
    /// edge as the active card above. Areas + wp-admin chip are dropped
    /// here on purpose — activation promotes the site to the top card.
    pub(super) fn render_compact_site_row(
        &self,
        site: &manage_sites::ManagedSiteInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sid = site.site_id.clone();
        let label = site.display_label();
        let brand = parse_brand_hex(&site.brand_color);
        let border_color = brand
            .map(|c| c.opacity(0.45))
            .unwrap_or(ShellDeckColors::border());
        let sid_for_click = sid.clone();
        let label_for_click = label.clone();

        div().w_full().h(px(SITE_ROW_H)).py(px(4.0)).child(
            div()
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(border_color)
                .bg(ShellDeckColors::bg_sidebar())
                .child({
                    let mut identity = div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden();
                    let mut label_row = div().flex().items_center().gap(px(6.0)).child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .truncate()
                            .child(label.clone()),
                    );
                    if site.is_wordpress == Some(true) {
                        label_row = label_row.child(
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::primary())
                                .flex_shrink_0()
                                .child("WP"),
                        );
                    }
                    identity = identity.child(label_row).child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .truncate()
                            .child(if site.host.is_empty() {
                                site.tenant_name.clone()
                            } else {
                                site.host.clone()
                            }),
                    );
                    identity
                })
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "uh-act-{}",
                            sid
                        ))))
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .flex_shrink_0()
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_primary())
                        .text_color(ShellDeckColors::text_primary())
                        .cursor_pointer()
                        .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                        .child(t!("user.sites.activate").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.select_site(
                                Some(sid_for_click.clone()),
                                Some(label_for_click.clone()),
                                cx,
                            );
                        })),
                ),
        )
    }

    /// Tab bar for the User-mode home. Same visual shape as
    /// `SupportView::render_section_tabs`
    /// (compact_filter_button + icon, `Default` variant when active).
    pub(super) fn render_user_home_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |label: String,
                   icon: &'static str,
                   target: UserHomeTab,
                   this_tab: UserHomeTab,
                   cx: &mut Context<Self>| {
            let active = this_tab == target;
            let entity = cx.entity();
            adabraka_ui::components::button::Button::new(
                ElementId::from(SharedString::from(format!("uh-tab-{target:?}"))),
                label,
            )
            .size(adabraka_ui::components::button::ButtonSize::Sm)
            .h(px(26.0))
            .px(px(10.0))
            .variant(if active {
                ButtonVariant::Default
            } else {
                ButtonVariant::Outline
            })
            .icon(IconSource::from(icon))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.user_home_tab = target;
                    cx.notify();
                });
            })
        };
        let current = self.user_home_tab;
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(16.0))
            .pt(px(4.0))
            .pb(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(tab(
                t!("user.tabs.home").to_string(),
                "house",
                UserHomeTab::Home,
                current,
                cx,
            ))
            .child(tab(
                t!("user.tabs.sites").to_string(),
                "globe",
                UserHomeTab::Sites,
                current,
                cx,
            ))
            .child(tab(
                t!("user.tabs.requests").to_string(),
                "tag",
                UserHomeTab::Requests,
                current,
                cx,
            ))
            .child(tab(
                t!("user.tabs.infos").to_string(),
                "user",
                UserHomeTab::Infos,
                current,
                cx,
            ))
    }

    pub(super) fn render_user_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sites = self
            .site_directory
            .as_ref()
            .map(|payload| payload.sites.len())
            .unwrap_or(0);
        // Keep the User dashboard personal even if an older Manage build or a
        // stale staff-scoped cache hands us a broader list.
        let my_requests = self
            .issues_list
            .iter()
            .filter(|issue| self.is_my_issue(issue))
            .cloned()
            .collect::<Vec<_>>();
        let open_requests = my_requests
            .iter()
            .filter(|issue| !issue.is_closed())
            .count();

        let stat = |icon: &'static str, value: usize, label: String| {
            adabraka_ui::display::card::Card::new()
                .content(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(
                            div()
                                .size(px(38.0))
                                .rounded(px(10.0))
                                .bg(ShellDeckColors::primary().opacity(0.12))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(lucide_icon(icon, 18.0, ShellDeckColors::primary())),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(24.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(value.to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(label),
                                ),
                        ),
                )
                .min_w(px(180.0))
                .flex_1()
        };

        let entity = cx.entity();
        let sites_action = Button::new("home-open-sites", t!("user.home.open_sites").to_string())
            .variant(ButtonVariant::Outline)
            .icon(IconSource::from("globe"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.user_home_tab = UserHomeTab::Sites;
                    cx.notify();
                });
            });
        let entity = cx.entity();
        let requests_action = Button::new(
            "home-open-requests",
            t!("user.home.open_requests").to_string(),
        )
        .variant(ButtonVariant::Outline)
        .icon(IconSource::from("tag"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.user_home_tab = UserHomeTab::Requests;
                cx.notify();
            });
        });
        let entity = cx.entity();
        let new_request = Button::new("home-new-request", t!("user.home.new_request").to_string())
            .icon(IconSource::from("plus"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.open_new_request(cx));
            });

        let recent_requests = if my_requests.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .min_h(px(132.0))
                .child(lucide_icon("inbox", 24.0, ShellDeckColors::text_muted()))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.home.recent_empty").to_string()),
                )
                .into_any_element()
        } else {
            let rows = my_requests
                .iter()
                .take(3)
                .cloned()
                .enumerate()
                .map(|(index, issue)| {
                    let issue_id = issue.id.clone();
                    let entity = cx.entity();
                    let updated = rel_time(issue.updated_at);
                    div()
                        .id(("home-recent-request", index))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .px(px(2.0))
                        .py(px(10.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border().opacity(0.65))
                        .cursor_pointer()
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.user_home_tab = UserHomeTab::Requests;
                                this.select_issue(issue_id.clone(), cx);
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .size(px(30.0))
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::primary().opacity(0.10))
                                .flex()
                                .items_center()
                                .justify_center()
                                .flex_shrink_0()
                                .child(lucide_icon("inbox", 14.0, ShellDeckColors::primary())),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .min_w(px(0.0))
                                .flex_1()
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .overflow_hidden()
                                        .child(issue.title),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(updated),
                                ),
                        )
                        .child(issue_status_badge(&issue.status))
                })
                .collect::<Vec<_>>();
            div().flex().flex_col().children(rows).into_any_element()
        };

        let account_status = match self.account_status {
            AccountStatus::Ok => t!("user.home.status.connected").to_string(),
            AccountStatus::Rejected => t!("user.home.status.expired").to_string(),
            AccountStatus::Offline => t!("user.home.status.offline").to_string(),
            AccountStatus::Unknown => t!("user.home.status.checking").to_string(),
        };
        let active_site = self
            .app_config
            .cloud_sync
            .active_site_label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| t!("user.home.no_active_site").to_string());
        let status_row = |icon: &'static str, label: String, value: String, color: Hsla| {
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(7.0))
                .child(
                    div()
                        .size(px(28.0))
                        .rounded(px(7.0))
                        .bg(color.opacity(0.10))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(lucide_icon(icon, 13.0, color)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(ShellDeckColors::text_primary())
                                .overflow_hidden()
                                .child(value),
                        ),
                )
        };
        let entity = cx.entity();
        let sync_action = Button::new("home-sync", t!("user.home.sync").to_string())
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("refresh-cw"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.cloud_sync_now(cx));
            });

        div()
            .id("user-overview-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .gap(px(16.0))
            .p(px(16.0))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(132.0))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .rounded(use_theme().tokens.radius_lg)
                    .border_1()
                    .border_color(ShellDeckColors::primary().opacity(0.40))
                    // Match the surrounding page so GPUI's rectangular
                    // background paint cannot show behind the curved border.
                    // The dark artwork itself stays safely inset below.
                    .bg(ShellDeckColors::bg_primary())
                    .child(
                        img("images/home/user-dashboard-colorful-watermark-v2.webp")
                            .absolute()
                            .inset_0()
                            .size_full()
                            // The asset is exported at this exact aspect ratio
                            // with its gradient and Card-equivalent alpha
                            // corners baked in, so GPUI has nothing to mask.
                            .object_fit(ObjectFit::Fill),
                    )
                    .child(
                        div()
                            .relative()
                            .ml_auto()
                            .w(relative(0.52))
                            .h_full()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .items_start()
                            .gap(px(7.0))
                            .px(px(24.0))
                            .child(
                                div()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded_full()
                                    .bg(ShellDeckColors::primary().opacity(0.22))
                                    .border_1()
                                    .border_color(ShellDeckColors::primary().opacity(0.45))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(hsla(0.47, 0.78, 0.72, 1.0))
                                    .child(
                                        t!("user.home.directory_count", count = sites).to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(21.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(white())
                                    .child(t!("user.home.title").to_string()),
                            )
                            .child(
                                div()
                                    .max_w(px(430.0))
                                    .text_size(px(12.0))
                                    .text_color(white().opacity(0.72))
                                    .child(t!("user.home.subtitle").to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(stat("globe", sites, t!("user.home.sites").to_string()))
                    .child(stat(
                        "inbox",
                        open_requests,
                        t!("user.home.open_requests_count").to_string(),
                    )),
            )
            .child(
                adabraka_ui::display::card::Card::new().content(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(t!("user.home.quick_actions").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(t!("user.home.quick_actions_hint").to_string()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_end()
                                .gap(px(8.0))
                                .child(sites_action)
                                .child(requests_action)
                                .child(new_request),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(12.0))
                    .child(
                        adabraka_ui::display::card::Card::new()
                            .content(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap(px(8.0))
                                            .pb(px(6.0))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(8.0))
                                                    .child(lucide_icon(
                                                        "tag",
                                                        15.0,
                                                        ShellDeckColors::primary(),
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_size(px(14.0))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(
                                                                ShellDeckColors::text_primary(),
                                                            )
                                                            .child(
                                                                t!("user.home.recent_requests")
                                                                    .to_string(),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(ShellDeckColors::text_muted())
                                                    .child(
                                                        t!("user.home.latest_three").to_string(),
                                                    ),
                                            ),
                                    )
                                    .child(recent_requests),
                            )
                            .min_w(px(300.0))
                            .flex_1(),
                    )
                    .child(
                        adabraka_ui::display::card::Card::new()
                            .content(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(5.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .pb(px(6.0))
                                            .child(lucide_icon(
                                                "activity",
                                                15.0,
                                                ShellDeckColors::primary(),
                                            ))
                                            .child(
                                                div()
                                                    .text_size(px(14.0))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(ShellDeckColors::text_primary())
                                                    .child(
                                                        t!("user.home.workspace_status")
                                                            .to_string(),
                                                    ),
                                            ),
                                    )
                                    .child(status_row(
                                        "shield",
                                        t!("user.home.account").to_string(),
                                        account_status,
                                        self.account_status.dot_color(),
                                    ))
                                    .child(status_row(
                                        "globe",
                                        t!("user.home.active_site").to_string(),
                                        active_site,
                                        ShellDeckColors::primary(),
                                    ))
                                    .child(status_row(
                                        "database",
                                        t!("user.home.directory").to_string(),
                                        t!("user.home.directory_count", count = sites).to_string(),
                                        ShellDeckColors::success(),
                                    ))
                                    .child(
                                        div().flex().justify_end().pt(px(6.0)).child(sync_action),
                                    ),
                            )
                            .min_w(px(260.0))
                            .flex_1(),
                    ),
            )
    }

    /// User-mode "Mes informations" tab — surfaces every field the
    /// `/whoami` payload returned (device label, created_at, last_seen_at,
    /// role) plus the account bits and directory stats. Deliberately
    /// read-only so it can't accidentally mutate credentials.
    pub(super) fn render_user_infos_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
        let server = self.account_base_url();
        let payload = self.site_directory.clone().unwrap_or_default();
        let whoami = self.last_whoami.clone().unwrap_or_default();

        // Small helper: one "field row" (label muted small, value primary
        // wrapping). Copies the shape of the ticket detail meta rows so
        // the visual language stays the same across surfaces.
        let field = |label: String, value: String, icon: &'static str| {
            div()
                .flex()
                .items_start()
                .gap(px(10.0))
                .py(px(8.0))
                .child(
                    div()
                        .size(px(28.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::primary().opacity(0.10))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .child(lucide_icon(icon, 13.0, ShellDeckColors::primary())),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_muted())
                                .child(label.to_uppercase()),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(if value.trim().is_empty() {
                                    t!("user.infos.unknown").to_string()
                                } else {
                                    value
                                }),
                        ),
                )
        };

        // Section chrome — same p/rounded/border/bg as other User-mode cards.
        let section = |title: String, icon: &'static str, body: gpui::Div| {
            div()
                .flex()
                .flex_col()
                .m(px(16.0))
                .mb(px(0.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(16.0))
                        .py(px(12.0))
                        .border_b_1()
                        .border_color(ShellDeckColors::border())
                        .child(lucide_icon(icon, 15.0, ShellDeckColors::primary()))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(title),
                        ),
                )
                .child(div().flex().flex_col().px(px(16.0)).py(px(4.0)).child(body))
        };

        let role_label = if account.is_superadmin {
            t!("user.infos.role.superadmin").to_string()
        } else if account.is_inklura_support {
            t!("user.infos.role.inklura_support").to_string()
        } else if account.is_admin {
            t!("user.infos.role.admin").to_string()
        } else {
            t!("user.infos.role.user").to_string()
        };

        // Session — device + role + timestamps returned by whoami.
        let session_body = div()
            .flex()
            .flex_col()
            .child(field(
                t!("user.infos.field.device").to_string(),
                whoami.label.clone().unwrap_or_default(),
                "keyboard",
            ))
            .child(field(
                t!("user.infos.field.role").to_string(),
                role_label,
                "shield",
            ))
            .child(field(
                t!("user.infos.field.since").to_string(),
                whoami.created_at.clone().unwrap_or_default(),
                "calendar",
            ))
            .child(field(
                t!("user.infos.field.last_seen").to_string(),
                whoami.last_seen_at.clone().unwrap_or_default(),
                "clock",
            ));

        // Account — identity + Manage server.
        let account_body = div()
            .flex()
            .flex_col()
            .child(field(
                t!("user.infos.field.name").to_string(),
                account.display_name(),
                "user",
            ))
            .child(field(
                t!("user.infos.field.email").to_string(),
                account.email.clone(),
                "mail",
            ))
            .child(field(
                t!("user.infos.field.server").to_string(),
                server,
                "globe",
            ));

        // Scope — tenant + sites the server exposed to us.
        let tenant_name = payload
            .sites
            .first()
            .map(|s| s.tenant_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_default();
        let sites_count = payload.sites.len();
        let scope_body = div()
            .flex()
            .flex_col()
            .child(field(
                t!("user.infos.field.tenant").to_string(),
                tenant_name,
                "users",
            ))
            .child(field(
                t!("user.infos.field.sites_available", count = sites_count).to_string(),
                t!("user.infos.field.sites_count", count = sites_count).to_string(),
                "globe",
            ));

        // Roles — one badge per entry in the CM role bag. Surfaces every
        // custom role (`content_editor`, `customer_service`, …) the tenant
        // admin defined in Manage, not just the hardcoded super-admin /
        // admin tiers the mode gate uses. See `.agents/roles.md` for the
        // "bag is the truth, predicates are shortcuts" rule.
        let roles_body = {
            let mut container = div().flex().flex_col().py(px(4.0));
            if account.roles.is_empty() {
                container = container.child(
                    div()
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.infos.roles.empty").to_string()),
                );
            } else {
                let mut row = div().flex().flex_wrap().gap(px(6.0)).py(px(8.0));
                for role in &account.roles {
                    row = row.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(8.0))
                            .py(px(3.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .border_1()
                            .border_color(ShellDeckColors::primary().opacity(0.35))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::primary())
                            .child(lucide_icon("shield", 10.0, ShellDeckColors::primary()))
                            .child(role.clone()),
                    );
                }
                container = container.child(row);
            }
            container
        };

        let _ = cx; // no listeners here — the tab is read-only.
        div()
            .id("user-infos-tab")
            .flex()
            .flex_col()
            .pb(px(16.0))
            .child(section(
                t!("user.infos.section.session").to_string(),
                "shield",
                session_body,
            ))
            .child(section(
                t!("user.infos.section.roles").to_string(),
                "shield",
                roles_body,
            ))
            .child(section(
                t!("user.infos.section.account").to_string(),
                "user",
                account_body,
            ))
            .child(section(
                t!("user.infos.section.scope").to_string(),
                "users",
                scope_body,
            ))
    }

    /// User mode: a manage-centric home — account header + "Mes sites" list with
    /// per-site Activer + area deep links.
    /// Pre-login welcome landing — intercepts the render whenever the user
    /// is not signed in (there is no guest path). Two-part layout:
    ///
    /// 1. **Hero** — ShellDeck brand icon + title + tagline + two CTAs
    ///    (sign in / create account).
    /// 2. **Inklura marketing** — the Inklura brand block + value props
    ///    lifted from inklura.fr, so a first-time visitor understands
    ///    what they're being invited into before creating an account.
    ///
    /// Kept inside a `scrollable_vertical` because on small windows the
    /// marketing block would push the CTAs offscreen.
    pub(super) fn render_welcome_screen(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Small helper for the four Inklura value-prop cards — same shape
        // so the row reads as a set.
        fn stat_card(icon: &'static str, value: String, label: String) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .w(px(150.0))
                .px(px(12.0))
                .py(px(14.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(lucide_icon(icon, 22.0, ShellDeckColors::primary()))
                .child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(value),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(label),
                )
        }

        let entity = cx.entity();

        // Hero — brand + CTAs.
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
            .pt(px(48.0))
            .pb(px(32.0))
            .child(
                // ShellDeck brand mark — PNG (not SVG) because GPUI renders
                // SVGs in currentColor and the mark's multi-fill palette
                // (teal frame + dark inner + light glyph) would collapse
                // to a single tint. The PNG raster preserves every colour.
                img("images/shelldeck-icon.png").w(px(72.0)).h(px(72.0)),
            )
            .child(
                div()
                    .text_size(px(24.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("welcome.title").to_string()),
            )
            .child(
                div()
                    .max_w(px(460.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("welcome.tagline").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .mt(px(8.0))
                    .child(
                        // Primary CTA — funnels to the existing LoginForm modal.
                        div()
                            .id("welcome-sign-in")
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(20.0))
                            .py(px(10.0))
                            .rounded(px(10.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("external-link"))
                                    .size(px(14.0))
                                    .text_color(white()),
                            )
                            .child(t!("welcome.sign_in").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.show_login_form(cx));
                                }
                            }),
                    )
                    .child(
                        // Secondary CTA — opens Manage signup in the browser.
                        div()
                            .id("welcome-signup")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(14.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .cursor_pointer()
                            .hover(|s| {
                                s.bg(ShellDeckColors::hover_bg())
                                    .text_color(ShellDeckColors::text_primary())
                            })
                            .child(lucide_icon(
                                "external-link",
                                11.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .child(t!("welcome.create_account").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.open_signup(cx));
                                }
                            }),
                    ),
            );

        // Inklura marketing block — content lifted from inklura.fr so the
        // messaging stays in sync with the marketing site. Not a full
        // marketing page; just enough for a first-time visitor to know
        // what they're being invited into.
        let inklura = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .mt(px(8.0))
            .pt(px(24.0))
            .pb(px(48.0))
            .px(px(32.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                // Inklura brand square — same 28×42 mark on #146BFF ground
                // as the login modal, for visual consistency across the
                // pre-auth surfaces.
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(28.0))
                    .h(px(42.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x146BFF))
                    .child(
                        svg()
                            .path("images/logo-inklura.svg")
                            .w(px(28.0))
                            .h(px(42.0))
                            .text_color(gpui::white()),
                    ),
            )
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("welcome.inklura.title").to_string()),
            )
            .child(
                div()
                    .max_w(px(560.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("welcome.inklura.subtitle").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap(px(10.0))
                    .mt(px(6.0))
                    .child(stat_card(
                        "zap",
                        t!("welcome.inklura.stat.savings.value").to_string(),
                        t!("welcome.inklura.stat.savings.label").to_string(),
                    ))
                    .child(stat_card(
                        "clock",
                        t!("welcome.inklura.stat.time.value").to_string(),
                        t!("welcome.inklura.stat.time.label").to_string(),
                    ))
                    .child(stat_card(
                        "shield",
                        t!("welcome.inklura.stat.uptime.value").to_string(),
                        t!("welcome.inklura.stat.uptime.label").to_string(),
                    ))
                    .child(stat_card(
                        "users",
                        t!("welcome.inklura.stat.clients.value").to_string(),
                        t!("welcome.inklura.stat.clients.label").to_string(),
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .mt(px(8.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("check", 11.0, ShellDeckColors::success()))
                    .child(t!("welcome.inklura.trust").to_string()),
            );

        // "Réalisé par WD29" footer — same shape as the Settings > About
        // signature so a first-time visitor sees the same attribution
        // whether they land here or hit About after signing in.
        const LOGO_H: f32 = 20.0;
        let made_by = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .py(px(20.0))
            .text_color(ShellDeckColors::text_muted())
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(LOGO_H))
                    .text_size(px(11.0))
                    .line_height(px(LOGO_H))
                    .child(t!("settings.about.made_by").to_string()),
            )
            .child(
                div().flex().items_center().h(px(LOGO_H)).child(
                    svg()
                        .path("images/wd29-logo.svg")
                        .w(px(56.0))
                        .h(px(LOGO_H))
                        .flex_shrink_0()
                        .text_color(ShellDeckColors::text_muted()),
                ),
            );

        // Full page — scrolls if the three blocks don't fit the window.
        div()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(scrollable_vertical(
                div()
                    .id("welcome-body")
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .w_full()
                            .child(hero)
                            .child(inklura)
                            .child(made_by),
                    ),
            ))
    }

    pub(super) fn render_user_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
        let server = self.account_base_url();
        let payload = self.site_directory.clone().unwrap_or_default();

        // Preferred area buttons for each site row (subset of the directory).
        let preferred = [
            "dashboard",
            "cms",
            "helpdesk",
            "ecommerce",
            "settings",
            "shelldeck",
        ];
        let area_buttons: Vec<manage_sites::ManageArea> = preferred
            .iter()
            .filter_map(|k| payload.areas.iter().find(|a| a.key == *k).cloned())
            .collect();

        // Header card.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .p(px(16.0))
            .m(px(16.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .size(px(40.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary().opacity(0.20))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::primary())
                            .child(account.initial()),
                    )
                    .child({
                        let mut name_row = div().flex().items_center().gap(px(8.0)).child(
                            div()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(account.display_name()),
                        );
                        // Super-admin badge (`shield` + label, primary tint)
                        // — surfaces the role the token was minted with so
                        // the user knows why they see Support/Dev options.
                        if account.is_superadmin {
                            name_row = name_row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(px(6.0))
                                    .bg(ShellDeckColors::primary().opacity(0.14))
                                    .border_1()
                                    .border_color(ShellDeckColors::primary().opacity(0.35))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::primary())
                                    .child(lucide_icon("shield", 10.0, ShellDeckColors::primary()))
                                    .child(t!("user.badge.super_admin").to_string()),
                            );
                        }
                        div().flex().flex_col().child(name_row).child(
                            div()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(format!("{} · {}", account.email, server)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("uh-open-manage")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("external-link"))
                                    .size(px(12.0))
                                    .text_color(white()),
                            )
                            .child(t!("user.open_manage").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.open_manage_area("/manage".to_string(), cx);
                            })),
                    )
                    .child(
                        div()
                            .id("uh-sync")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .bg(ShellDeckColors::bg_primary())
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .cursor_pointer()
                            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                            .child(lucide_icon(
                                "refresh-cw",
                                12.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .child(t!("user.sync").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.cloud_sync_now(cx);
                            })),
                    ),
            );

        // Sites: filter by search, sort (conn-bearing first, then alpha),
        // split into (active-card, others-for-virt-list). Recomputed inside
        // the `uniform_list` processor as well — cheap enough on 300 sites
        // (< 1ms) and keeps the model authoritative.
        let (active_site, others_sites) = self.partition_user_sites(cx);
        let others_count = others_sites.len();

        let mut list = div()
            .id("user-home-sites")
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0));

        if active_site.is_none() && others_count == 0 {
            // Centered CTA card instead of a passive mumble line — makes it
            // clear the next action is to open Manage (or Synchroniser if the
            // sites were just created).
            let empty_card = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .p(px(28.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(
                    div()
                        .size(px(44.0))
                        .rounded_full()
                        .bg(ShellDeckColors::primary().opacity(0.15))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(20.0))
                        .text_color(ShellDeckColors::primary())
                        .child(">_"),
                )
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("user.sites.empty.title").to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.sites.empty.hint").to_string()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .mt(px(4.0))
                        .child(
                            div()
                                .id("uh-empty-open-manage")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(14.0))
                                .py(px(8.0))
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::primary())
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(white())
                                .cursor_pointer()
                                .child(
                                    svg()
                                        .path(lucide_path("external-link"))
                                        .size(px(12.0))
                                        .text_color(white()),
                                )
                                .child(t!("user.open_manage").to_string())
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.open_manage_area("/manage".to_string(), cx);
                                })),
                        )
                        .child(
                            div()
                                .id("uh-empty-sync")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(14.0))
                                .py(px(8.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(ShellDeckColors::border())
                                .bg(ShellDeckColors::bg_primary())
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .cursor_pointer()
                                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                                .child(lucide_icon(
                                    "refresh-cw",
                                    12.0,
                                    ShellDeckColors::text_muted(),
                                ))
                                .child(t!("user.sync").to_string())
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.cloud_sync_now(cx);
                                })),
                        ),
                );
            list = list.child(empty_card);
        }

        // Active site sits at the top as a full "rich" card (identity +
        // wp-admin shortcut + all six area deep-links). It's the only card
        // that owns the areas — Activer on any other row promotes that
        // site here.
        if let Some(site) = active_site.as_ref() {
            list = list.child(self.render_active_site_card(site, &area_buttons, cx));
        }

        // Everyone else is a fixed-height compact row inside a virtualised
        // `uniform_list`. Height per row is deliberately uniform so GPUI's
        // virtualiser knows how many rows fit the viewport without probing
        // each one — that's the whole point of this refactor: paint budget
        // becomes O(visible) instead of O(sites).
        if others_count > 0 {
            const MAX_LIST_H: f32 = 600.0;
            const MIN_LIST_H: f32 = 120.0;
            let visible_h = (others_count as f32 * SITE_ROW_H).clamp(MIN_LIST_H, MAX_LIST_H);
            list = list.child(
                div().w_full().h(px(visible_h)).child(
                    uniform_list(
                        "user-home-sites-virt",
                        others_count,
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            let (_, others) = this.partition_user_sites(cx);
                            let mut items: Vec<AnyElement> = Vec::new();
                            for i in range {
                                if let Some(site) = others.get(i) {
                                    items.push(
                                        this.render_compact_site_row(site, cx).into_any_element(),
                                    );
                                }
                            }
                            items
                        }),
                    )
                    .w_full()
                    .h_full(),
                ),
            );
        }

        // Page body: account header, "Mes sites" section, optional Jean card,
        // "Mes demandes" section. Everything stacks at natural height; the
        // whole page scrolls if the content overflows.
        let tab = self.user_home_tab;
        let tab_bar = self.render_user_home_tab_bar(cx);

        // Body composition: header (persistent) + tab bar + tab content.
        // Each tab owns its own inner scroll. Previously the whole page
        // scrolled as one; splitting kept the header visible while the
        // active tab scrolls, and let the Sites tab embed a virtualised
        // list without competing with an outer scroll.
        let mut body = div()
            .id("user-home-body")
            .flex()
            .flex_col()
            .pb(px(24.0))
            .child(header)
            .child(tab_bar);
        match tab {
            UserHomeTab::Home => {
                body = body.child(self.render_user_overview(cx));
            }
            UserHomeTab::Sites => {
                body = body
                    .child({
                        // Section header: title on the left, live search on
                        // the right (only when there are enough sites to
                        // make it worth it — small tenants keep the row
                        // uncluttered).
                        let mut row = div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .px(px(16.0))
                            .pt(px(8.0))
                            .pb(px(6.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(lucide_icon(
                                        "globe",
                                        16.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(18.0))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("user.sites.title").to_string()),
                                    ),
                            );
                        if payload.sites.len() > 5 {
                            let entity = cx.entity();
                            row = row.child(
                                div().w(px(260.0)).child(
                                    Input::new(&self.user_sites_search_state)
                                        .size(InputSize::Sm)
                                        .placeholder(t!("user.sites.search").to_string())
                                        .prefix(lucide_icon(
                                            "search",
                                            12.0,
                                            ShellDeckColors::text_muted(),
                                        ))
                                        .on_change(move |_, cx| {
                                            entity.update(cx, |_, cx| cx.notify());
                                        }),
                                ),
                            );
                        }
                        row
                    })
                    .child(list)
                    .children(if self.has_jean() {
                        Some(self.render_jean_ask_card(cx))
                    } else {
                        None
                    });
            }
            UserHomeTab::Requests => {
                body = body.child(self.render_user_requests(cx));
            }
            UserHomeTab::Infos => {
                body = body.child(self.render_user_infos_tab(cx));
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(scrollable_vertical(body))
    }
}
