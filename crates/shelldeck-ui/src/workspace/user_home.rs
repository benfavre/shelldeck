use super::*;
use crate::overlay::round_window_bottom;

const WELCOME_CENTERED_MIN_LOGICAL_HEIGHT: f32 = 560.0;
const USER_HEADER_STACK_MAX_LOGICAL_WIDTH: f32 = 600.0;

pub(super) fn welcome_uses_compact_flow(viewport_height: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_height / scale < WELCOME_CENTERED_MIN_LOGICAL_HEIGHT
}

pub(super) fn user_header_uses_compact_flow(viewport_width: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_width / scale <= USER_HEADER_STACK_MAX_LOGICAL_WIDTH
}

fn managed_site_public_url(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let candidate = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{host}")
    };
    let parsed = url::Url::parse(&candidate).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed.to_string())
}

fn user_role_tokens(account: &cloud_account::AccountInfo) -> Vec<String> {
    let mut roles = Vec::new();
    for raw in &account.roles {
        let role = raw.trim().to_lowercase();
        if !role.is_empty() && !roles.iter().any(|existing| existing == &role) {
            roles.push(role);
        }
    }
    if roles.is_empty() {
        let fallback = if account.is_superadmin {
            "superadmin"
        } else if account.is_inklura_support {
            "inklura_support"
        } else if account.is_admin {
            "admin"
        } else {
            "user"
        };
        roles.push(fallback.to_string());
    }
    roles
}

fn humanize_custom_role(role: &str) -> String {
    let words = role
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

fn user_role_label(role: &str) -> String {
    match role {
        "superadmin" | "super_admin" => t!("user.infos.role.superadmin").to_string(),
        "inklura_support" => t!("user.infos.role.inklura_support").to_string(),
        "admin" | "administrator" => t!("user.infos.role.admin").to_string(),
        "tenant_admin" => t!("user.infos.role.tenant_admin").to_string(),
        "owner" => t!("user.infos.role.owner").to_string(),
        "user" => t!("user.infos.role.user").to_string(),
        custom => humanize_custom_role(custom),
    }
}

fn primary_user_role(roles: &[String]) -> Option<&str> {
    const PRIORITY: [&str; 7] = [
        "superadmin",
        "super_admin",
        "inklura_support",
        "admin",
        "administrator",
        "tenant_admin",
        "owner",
    ];
    PRIORITY
        .into_iter()
        .find(|candidate| roles.iter().any(|role| role == candidate))
        .or_else(|| roles.first().map(String::as_str))
}

fn nonempty_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl Workspace {
    fn open_user_site_url(&mut self, url: String, cx: &mut Context<Self>) {
        match cloud_account::open_in_browser(&url) {
            Ok(_) => self.show_toast(
                t!("toast.opening_browser").to_string(),
                ToastLevel::Info,
                cx,
            ),
            Err(e) => self.show_toast(
                t!(
                    "toast.open_browser_failed",
                    error = crate::i18n::api_error_message(&e)
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
        }
    }

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
        self.open_user_site_url(url, cx);
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

        // Row 2: public site, Manage, wp-admin (if any) + area deep-links.
        let mut areas_row = div().flex().flex_wrap().gap(px(6.0));
        if let Some(public_url) = managed_site_public_url(&site.host) {
            areas_row = areas_row.child(
                Button::new(
                    SharedString::from(format!("uh-open-public-{sid}")),
                    t!("user.sites.open_public").to_string(),
                )
                .variant(ButtonVariant::Outline)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("external-link"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_user_site_url(public_url.clone(), cx);
                })),
            );
        }
        let manage_site = site.clone();
        areas_row = areas_row.child(
            Button::new(
                SharedString::from(format!("uh-open-manage-{sid}")),
                t!("user.sites.open_manage").to_string(),
            )
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("settings"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_area_for_site(manage_site.clone(), "/manage/sites".to_string(), cx);
            })),
        );
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
        let public_url = managed_site_public_url(&site.host);
        let manage_site = site.clone();

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
                .child({
                    let mut actions = div().flex().items_center().gap(px(4.0)).flex_shrink_0();
                    if let Some(public_url) = public_url {
                        actions = actions.child(
                            Button::new(SharedString::from(format!("uh-row-public-{sid}")), "")
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Icon)
                                .icon(IconSource::from("external-link"))
                                .tooltip(t!("user.sites.open_public").to_string())
                                .w(px(28.0))
                                .h(px(28.0))
                                .px(px(0.0))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_user_site_url(public_url.clone(), cx);
                                })),
                        );
                    }
                    actions = actions.child(
                        Button::new(SharedString::from(format!("uh-row-manage-{sid}")), "")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Icon)
                            .icon(IconSource::from("settings"))
                            .tooltip(t!("user.sites.open_manage").to_string())
                            .w(px(28.0))
                            .h(px(28.0))
                            .px(px(0.0))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_area_for_site(
                                    manage_site.clone(),
                                    "/manage/sites".to_string(),
                                    cx,
                                );
                            })),
                    );
                    actions.child(
                        Button::new(
                            SharedString::from(format!("uh-choose-{sid}")),
                            t!("user.sites.choose").to_string(),
                        )
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_site(
                                Some(sid_for_click.clone()),
                                Some(label_for_click.clone()),
                                cx,
                            );
                        })),
                    )
                }),
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
        // A successful `mine=1` response is authoritative even if Manage uses
        // another `requested_by` representation. Until then, keep the User
        // dashboard defensive against a stale staff-scoped cache.
        let my_requests = self
            .issues_list
            .iter()
            .filter(|issue| self.is_user_visible_issue(issue))
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
                                        .child(crate::external_content::external_title(
                                            &issue.title,
                                        )),
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
        // Accord du compteur de sites : le projet sépare `.one` et `.many`
        // plutôt que d'écrire « 1 sites » (cf. `tray.counter.*`).
        let directory_count = if sites == 1 {
            t!("user.home.directory_count.one").to_string()
        } else {
            t!("user.home.directory_count.many", count = sites).to_string()
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
                    // The artwork itself stays safely inset below.
                    .bg(ShellDeckColors::bg_primary())
                    .child(
                        img("images/home/user-dashboard-colorful-v2.webp")
                            .absolute()
                            .inset_0()
                            .size_full()
                            // Preserve the illustration at every window width.
                            // Branding is a sibling below, never baked into or
                            // stretched with the raster artwork.
                            .rounded(use_theme().tokens.radius_lg)
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        // This is intentionally a real UI element rather than
                        // pixels in the hero image: resizing may crop the
                        // landscape, but it must never distort the wordmark.
                        div()
                            .absolute()
                            .left(px(18.0))
                            .bottom(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(
                                svg()
                                    .path("images/shelldeck-mark.svg")
                                    .size(px(22.0))
                                    .flex_shrink_0()
                                    .text_color(white().opacity(0.90)),
                            )
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(white().opacity(0.90))
                                    .child("ShellDeck"),
                            ),
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
                            // Marge gauche plus généreuse que la droite : le
                            // bord du champ bleu vit dans l'illustration et se
                            // déplace avec le recadrage, on s'en écarte.
                            .pl(px(44.0))
                            .pr(px(24.0))
                            .child(
                                div()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded_full()
                                    // Fond sombre plein, et non une teinte à
                                    // 22 % : le champ bleu de la bannière est
                                    // peint dans l'illustration, dont le
                                    // recadrage `Cover` déplace le bord selon
                                    // la largeur de fenêtre. À certaines
                                    // tailles la pastille retombait sur la
                                    // pâte à modeler claire, et son texte
                                    // disparaissait.
                                    .bg(ShellDeckColors::backdrop())
                                    .border_1()
                                    .border_color(ShellDeckColors::primary().opacity(0.55))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(hsla(0.47, 0.78, 0.72, 1.0))
                                    .child(directory_count.clone()),
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
                                        directory_count.clone(),
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

    /// User-mode "Mes informations" tab — surfaces the meaningful fields the
    /// `/whoami` payload returned plus the account bits and directory stats.
    /// Missing optional values are omitted rather than rendered as dashes.
    /// Deliberately read-only so it can't accidentally mutate credentials.
    pub(super) fn render_user_infos_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
        let server = self.account_base_url();
        let payload = self.site_directory.clone().unwrap_or_default();
        let role_tokens = user_role_tokens(&account);
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
                                .child(value),
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

        // Session — only values actually returned by whoami. Empty optional
        // timestamps must not turn into rows whose value is just an em dash.
        let mut session_body = div().flex().flex_col();
        if let Some(device) = whoami.label.as_deref().and_then(nonempty_text) {
            session_body = session_body.child(field(
                t!("user.infos.field.device").to_string(),
                device,
                "keyboard",
            ));
        }
        if let Some(created_at) = whoami.created_at.as_deref().and_then(nonempty_text) {
            session_body = session_body.child(field(
                t!("user.infos.field.since").to_string(),
                crate::i18n::local_timestamp(&created_at),
                "calendar",
            ));
        }
        if let Some(last_seen_at) = whoami.last_seen_at.as_deref().and_then(nonempty_text) {
            session_body = session_body.child(field(
                t!("user.infos.field.last_seen").to_string(),
                crate::i18n::local_timestamp(&last_seen_at),
                "clock",
            ));
        }

        // Account — identity + Manage server.
        let mut account_body = div().flex().flex_col();
        if let Some(name) = nonempty_text(&account.display_name()) {
            account_body =
                account_body.child(field(t!("user.infos.field.name").to_string(), name, "user"));
        }
        if let Some(email) = nonempty_text(&account.email) {
            account_body = account_body.child(field(
                t!("user.infos.field.email").to_string(),
                email,
                "mail",
            ));
        }
        if let Some(server) = nonempty_text(&server) {
            account_body = account_body.child(field(
                t!("user.infos.field.server").to_string(),
                server,
                "globe",
            ));
        }

        // Scope — tenant + sites the server exposed to us.
        let tenant_name = payload
            .sites
            .first()
            .map(|s| s.tenant_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_default();
        let sites_count = payload.sites.len();
        let mut scope_body = div().flex().flex_col();
        if let Some(tenant_name) = nonempty_text(&tenant_name) {
            scope_body = scope_body.child(field(
                t!("user.infos.field.tenant").to_string(),
                tenant_name,
                "users",
            ));
        }
        scope_body = scope_body.child(field(
            t!("user.infos.field.sites_available", count = sites_count).to_string(),
            t!("user.infos.field.sites_count", count = sites_count).to_string(),
            "globe",
        ));

        // Access — one human label per entry in the CM role bag, including
        // custom roles. Explicit capability flags supply one coherent fallback
        // only for legacy tokens whose bag is absent; they are never merged
        // into a non-empty bag. See `.agents/roles.md`.
        let roles_body = {
            let mut row = div().flex().flex_wrap().gap(px(6.0)).py(px(8.0));
            for role in &role_tokens {
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
                        .child(user_role_label(role)),
                );
            }
            div().flex().flex_col().py(px(4.0)).child(row)
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
    /// per-site selection plus public-site and Manage deep links.
    /// Pre-login welcome landing — intercepts the render whenever the user
    /// is not signed in (there is no guest path). Two-part layout:
    ///
    /// **Hero** — ShellDeck brand icon + one product promise + two CTAs
    /// (sign in / create account). The installed app is an authentication
    /// surface, not a second marketing landing page: broader Inklura claims
    /// belong on the public website.
    ///
    /// Kept inside a `scrollable_vertical` because on small windows the
    /// hero and publisher attribution must remain reachable.
    /// `is_maximized` : comme le mode Utilisateur, l'écran de bienvenue est la
    /// couche opaque la plus basse — la barre d'état n'est pas montée ici.
    pub(super) fn render_welcome_screen(
        &self,
        is_maximized: bool,
        compact_height: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();

        // Hero — brand + CTAs.
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.0))
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
                    .text_center()
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

        // "Réalisé par WD29" footer — same shape as the Settings > About
        // signature so a first-time visitor sees the same attribution
        // whether they land here or hit About after signing in.
        const LOGO_H: f32 = 20.0;
        let made_by = div()
            .flex()
            .flex_shrink_0()
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

        // Normal windows center the actual task and pin attribution to the
        // bottom. At the 400 px application minimum the same blocks switch to
        // one scrolling column, so neither CTA nor the publisher disappears.
        let content = if compact_height {
            scrollable_vertical(
                div()
                    .id("welcome-body")
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .py(px(32.0))
                    .child(hero)
                    .child(made_by),
            )
            .into_any_element()
        } else {
            div()
                .id("welcome-body")
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h(px(0.0))
                        .w_full()
                        .items_center()
                        .justify_center()
                        .child(hero),
                )
                .child(made_by)
                .into_any_element()
        };

        round_window_bottom(
            div()
                .size_full()
                .bg(ShellDeckColors::bg_primary())
                .overflow_hidden(),
            is_maximized,
        )
        .child(content)
    }

    /// `is_maximized` sert uniquement aux deux coins bas : cette surface est la
    /// couche opaque la plus basse du mode Utilisateur, il n'y a pas de barre
    /// d'état sous elle pour les porter.
    pub(super) fn render_user_home(
        &self,
        is_maximized: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
        let payload = self.site_directory.clone().unwrap_or_default();
        let role_tokens = user_role_tokens(&account);
        let primary_role = primary_user_role(&role_tokens)
            .filter(|role| *role != "user")
            .map(user_role_label);
        let compact_header = user_header_uses_compact_flow(
            window.viewport_size().width.to_f64() as f32,
            self.ui_font_size,
        );

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
        let mut header = div()
            .flex()
            .gap(px(12.0))
            .p(px(16.0))
            .m(px(16.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar());
        if compact_header {
            header = header.flex_col();
        } else {
            header = header.items_center().justify_between();
        }
        let header = header
            .child({
                let mut identity_group = div().flex().items_center().gap(px(12.0));
                if compact_header {
                    identity_group = identity_group.w_full();
                }
                identity_group
                    .child(
                        div()
                            .size(px(40.0))
                            .flex_shrink_0()
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
                        // The header consumes the same role presentation as
                        // Mes informations. A contradictory legacy payload can
                        // therefore never show two different access levels.
                        if let Some(role_label) = primary_role.clone() {
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
                                    .child(role_label),
                            );
                        }
                        let mut identity = div().flex().flex_col().child(name_row);
                        if let Some(email) = nonempty_text(&account.email) {
                            identity = identity.child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(email),
                            );
                        }
                        identity
                    })
            })
            .child({
                let mut actions = div().flex().items_center().flex_shrink_0().gap(px(8.0));
                if compact_header {
                    actions = actions.w_full().flex_wrap().justify_end();
                }
                actions
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
                    )
            });

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
        // public/Manage shortcuts + all six area deep-links). It's the only
        // card that owns the areas — choosing any other row promotes that site
        // here.
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

        // Page body: account header, "Mes sites" section, optional Monique card,
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
                    .children(if self.has_monique() {
                        Some(self.render_monique_ask_card(cx))
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

        round_window_bottom(
            div()
                .size_full()
                .flex()
                .flex_col()
                .bg(ShellDeckColors::bg_primary())
                .overflow_hidden(),
            is_maximized,
        )
        .child(scrollable_vertical(body))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        humanize_custom_role, managed_site_public_url, nonempty_text, primary_user_role,
        user_header_uses_compact_flow, user_role_tokens, welcome_uses_compact_flow,
    };
    use shelldeck_core::config::cloud_account::AccountInfo;

    // SDTEST-1708 — SDUC-441
    #[test]
    fn sdtest_1708_welcome_height_breakpoint_tracks_ui_scale() {
        assert!(welcome_uses_compact_flow(559.0, 14.0));
        assert!(!welcome_uses_compact_flow(560.0, 14.0));
        assert!(welcome_uses_compact_flow(1_119.0, 28.0));
        assert!(!welcome_uses_compact_flow(1_120.0, 28.0));
    }

    // SDTEST-1728 — SDUC-440. The account header stacks before either action
    // can leave the card, and the threshold follows the configured UI scale.
    #[test]
    fn user_account_header_breakpoint_tracks_ui_scale() {
        assert!(user_header_uses_compact_flow(600.0, 14.0));
        assert!(!user_header_uses_compact_flow(601.0, 14.0));
        assert!(!user_header_uses_compact_flow(700.0, 14.0));
        assert!(user_header_uses_compact_flow(1_200.0, 28.0));
        assert!(!user_header_uses_compact_flow(1_201.0, 28.0));
    }

    // SDTEST-1716 — remote site metadata is allowed to omit the scheme, but
    // it must never turn the browser action into a credential-bearing or
    // non-HTTP destination.
    #[test]
    fn managed_site_public_links_are_normalized_and_http_only() {
        assert_eq!(
            managed_site_public_url("boutique.example.test"),
            Some("https://boutique.example.test/".to_string())
        );
        assert_eq!(
            managed_site_public_url("http://boutique.example.test/catalogue"),
            Some("http://boutique.example.test/catalogue".to_string())
        );
        assert_eq!(managed_site_public_url(""), None);
        assert_eq!(managed_site_public_url("javascript://alert"), None);
        assert_eq!(managed_site_public_url("https://user@example.test"), None);
    }

    // SDTEST-1717 — Mes informations must not combine the capability flags
    // with a different role bag. The bag wins when present; legacy tokens get
    // one coherent fallback, and absent optional values disappear entirely.
    #[test]
    fn user_information_uses_one_role_source_and_hides_empty_values() {
        let contradictory = AccountInfo {
            is_superadmin: true,
            roles: vec![
                " tenant_admin ".into(),
                "CONTENT_EDITOR".into(),
                "tenant_admin".into(),
            ],
            ..Default::default()
        };
        let roles = user_role_tokens(&contradictory);
        assert_eq!(roles, ["tenant_admin", "content_editor"]);
        assert_eq!(primary_user_role(&roles), Some("tenant_admin"));
        assert_eq!(humanize_custom_role("content_editor"), "Content editor");

        let legacy_staff = AccountInfo {
            is_superadmin: true,
            ..Default::default()
        };
        assert_eq!(user_role_tokens(&legacy_staff), ["superadmin"]);
        assert_eq!(user_role_tokens(&AccountInfo::default()), ["user"]);

        assert_eq!(nonempty_text("  "), None);
        assert_eq!(
            nonempty_text("  Poste de Karim  "),
            Some("Poste de Karim".into())
        );
    }
}
