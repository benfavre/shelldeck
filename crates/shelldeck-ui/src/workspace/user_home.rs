use super::*;
use crate::overlay::round_window_bottom;
use std::rc::Rc;

const WELCOME_CENTERED_MIN_LOGICAL_HEIGHT: f32 = 560.0;
const USER_HOME_COMPACT_MAX_LOGICAL_WIDTH: f32 = 600.0;
/// From this width the home banner subtitle fits on one line, so the banner
/// can drop to its short height.
const USER_HOME_WIDE_BANNER_MIN_LOGICAL_WIDTH: f32 = 1100.0;

pub(super) fn welcome_uses_compact_flow(viewport_height: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_height / scale < WELCOME_CENTERED_MIN_LOGICAL_HEIGHT
}

pub(super) fn user_home_uses_compact_flow(viewport_width: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_width / scale <= USER_HOME_COMPACT_MAX_LOGICAL_WIDTH
}

pub(super) fn user_home_uses_wide_banner(viewport_width: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_width / scale >= USER_HOME_WIDE_BANNER_MIN_LOGICAL_WIDTH
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserSitesEmptyState {
    AccountEmpty,
    Filtered,
}

fn user_sites_empty_state(
    total_sites: usize,
    visible_sites: usize,
    query: &str,
) -> Option<UserSitesEmptyState> {
    if visible_sites > 0 {
        None
    } else if total_sites == 0 {
        Some(UserSitesEmptyState::AccountEmpty)
    } else if !query.trim().is_empty() {
        Some(UserSitesEmptyState::Filtered)
    } else {
        None
    }
}

/// Background of the User home page: the page colour with a breath of the
/// brand colour, so the panels stand out on it. Opaque on purpose: the banner
/// paints it again over its own square corner (see `render_user_overview`).
fn user_home_page_bg() -> Hsla {
    ShellDeckColors::bg_primary().blend(ShellDeckColors::primary().opacity(0.045))
}

/// Corner radius of a banner that carries the street scene.
const BANNER_RADIUS: f32 = 16.0;

/// A panel of the User home: the lightest surface, a quiet border, a soft
/// shadow and generous corners.
fn home_panel() -> Div {
    div()
        .rounded(px(14.0))
        .border_1()
        .border_color(ShellDeckColors::border().opacity(0.55))
        .bg(ShellDeckColors::bg_sidebar())
        .shadow(vec![use_theme().tokens.shadow_sm].into())
}

/// The tinted tile that carries the icon of a counter or of a row.
fn home_icon_tile(icon: &'static str, color: Hsla, round: bool) -> Div {
    let tile = div()
        .size(px(36.0))
        .flex_shrink_0()
        .bg(color.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(lucide_icon(icon, 17.0, color));
    if round {
        tile.rounded_full()
    } else {
        tile.rounded(px(9.0))
    }
}

/// Title of a panel: its icon in the brand colour, then the label.
fn home_section_title(icon: &'static str, label: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(lucide_icon(icon, 18.0, ShellDeckColors::primary()))
        .child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(label),
        )
}

/// How the status of a request is shown on the home page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeStatusTone {
    /// Waiting for someone: the one that should catch the eye.
    Waiting,
    /// Being worked on.
    Active,
    /// Being looked at before work starts.
    Review,
    Blocked,
    /// Done or closed: nothing left to do.
    Settled,
}

fn home_status_tone(status: &str) -> HomeStatusTone {
    match status {
        "open" => HomeStatusTone::Waiting,
        "in_progress" => HomeStatusTone::Active,
        "blocked" => HomeStatusTone::Blocked,
        "done" | "closed" => HomeStatusTone::Settled,
        _ => HomeStatusTone::Review,
    }
}

/// First letter in upper case: status labels are written in lower case for
/// running text, and a pill is a label of its own.
fn capitalize_first(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Status of a request as a pill, coloured by what it asks of the reader.
fn home_status_pill(status: &str) -> Div {
    let pill = div()
        .flex_shrink_0()
        .px(px(12.0))
        .py(px(4.0))
        .rounded_full()
        .whitespace_nowrap()
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(capitalize_first(&crate::support_view::issue_status_label(
            status,
        )));
    match home_status_tone(status) {
        HomeStatusTone::Waiting => pill
            .bg(ShellDeckColors::warning().opacity(0.42))
            .text_color(ShellDeckColors::text_primary()),
        HomeStatusTone::Active => pill
            .bg(ShellDeckColors::primary())
            .text_color(use_theme().tokens.primary_foreground),
        HomeStatusTone::Review => pill
            .bg(ShellDeckColors::primary().opacity(0.14))
            .text_color(ShellDeckColors::primary()),
        HomeStatusTone::Blocked => pill
            .bg(ShellDeckColors::error().opacity(0.16))
            .text_color(ShellDeckColors::error()),
        HomeStatusTone::Settled => pill
            .border_1()
            .border_color(ShellDeckColors::border())
            .text_color(ShellDeckColors::text_muted()),
    }
}

/// Keyboard-semantic wrapper for rich rows that cannot be represented by the
/// label-and-icon-only shared Button without losing their information layout.
type KeyboardActionHandler = dyn Fn(&mut Window, &mut App);

#[derive(IntoElement)]
pub(super) struct KeyboardAction {
    id: ElementId,
    base: Stateful<Div>,
    on_activate: Rc<KeyboardActionHandler>,
}

struct KeyboardActionFocusState {
    handle: FocusHandle,
    _focus_in: Subscription,
    _focus_out: Subscription,
}

impl KeyboardActionFocusState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let handle = cx.focus_handle();
        let state_for_focus = cx.weak_entity();
        let focus_in = window.on_focus_in(&handle, cx, move |_, cx| {
            let _ = state_for_focus.update(cx, |_, cx| cx.notify());
        });
        let state_for_blur = cx.weak_entity();
        let focus_out = window.on_focus_out(&handle, cx, move |_, _, cx| {
            let _ = state_for_blur.update(cx, |_, cx| cx.notify());
        });
        Self {
            handle,
            _focus_in: focus_in,
            _focus_out: focus_out,
        }
    }
}

impl KeyboardAction {
    pub(super) fn new(
        id: impl Into<ElementId>,
        base: Stateful<Div>,
        on_activate: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            base,
            on_activate: Rc::new(on_activate),
        }
    }
}

impl RenderOnce for KeyboardAction {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus_handle = window
            .use_keyed_state(self.id, cx, KeyboardActionFocusState::new)
            .read(cx)
            .handle
            .clone();
        let is_focused = focus_handle.is_focused(window);
        let theme = use_theme();
        let activate = self.on_activate;

        self.base
            .track_focus(&focus_handle.tab_index(0).tab_stop(true))
            .relative()
            .when(is_focused, |action| {
                action.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(theme.tokens.radius_md)
                        .border_2()
                        .border_color(theme.tokens.ring),
                )
            })
            // Match shared Button: mouse activation does not leave a focus
            // ring behind, while Tab focus remains visible.
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                window.prevent_default();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                activate(window, cx);
            })
    }
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
            .w_full()
            .min_w(px(0.0))
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(4.0))
            .truncate()
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
        let Some(payload) = self.site_directory.as_ref() else {
            return (None, Vec::new());
        };
        let active_id = self.app_config.cloud_sync.active_site_id.as_deref();
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
        // Cache the normalized label once per site. `sort_by` used to rebuild
        // both lowercase strings for every comparison (O(n log n) temporary
        // allocations) each time the Sites tab rendered.
        sites.sort_by_cached_key(|site| {
            (
                !conn_site_ids.contains(&site.site_id),
                site.display_label().to_lowercase(),
            )
        });
        let active = active_id
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
                Button::new(SharedString::from(format!("uh-wp-{sid}")), "wp-admin")
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("external-link"))
                    .h(px(28.0))
                    .px(px(8.0))
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, _cx| {
                        let _ =
                            shelldeck_core::config::cloud_account::open_in_browser(&wp_url_owned);
                    })),
            );
        }
        for area in area_buttons {
            let site_clone = site.clone();
            let path = area.path.clone();
            let mut chip = Button::new(
                SharedString::from(format!("uh-area-{}-{}", sid, area.key)),
                area.label.clone(),
            )
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .h(px(28.0))
            .px(px(8.0))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_area_for_site(site_clone.clone(), path.clone(), cx);
            }));
            if let Some(slug) = manage_area_icon(&area.key) {
                chip = chip.icon(IconSource::from(slug));
            }
            areas_row = areas_row.child(chip);
        }
        card.child(areas_row)
    }

    /// Fixed-height compact row for a non-active site. The wide composition
    /// uses a 64 px slot; the compact User-home composition uses 88 px so the
    /// identity owns the first level and the three actions own the second.
    /// Both retain 4 px padding top/bottom, preserving the visual row gap
    /// without breaking `uniform_list`'s uniform-height contract.
    /// Width fills the parent (`w_full`) so rows land on the same right
    /// edge as the active card above. Areas + wp-admin chip are dropped
    /// here on purpose — activation promotes the site to the top card.
    pub(super) fn render_compact_site_row(
        &self,
        site: &manage_sites::ManagedSiteInfo,
        compact: bool,
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

        let row_h = if compact {
            SITE_ROW_H_COMPACT
        } else {
            SITE_ROW_H
        };
        let mut label_text = div()
            .min_w(px(0.0))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_primary());
        label_text = if compact {
            label_text.line_clamp(1)
        } else {
            label_text.truncate()
        };
        let mut label_row = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(label_text.child(label.clone()));
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
        let mut identity = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(label_row)
            .child(
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
        if compact {
            identity = identity.w_full();
        } else {
            identity = identity.flex_1();
        }

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
                    this.open_area_for_site(manage_site.clone(), "/manage/sites".to_string(), cx);
                })),
        );
        actions = actions.child(
            Button::new(
                SharedString::from(format!("uh-choose-{sid}")),
                t!("user.sites.choose").to_string(),
            )
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            // Match the two neighboring icon actions. The default small
            // button is taller and makes the whole action row sit
            // optically against the card's bottom edge.
            .h(px(28.0))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_site(
                    Some(sid_for_click.clone()),
                    Some(label_for_click.clone()),
                    cx,
                );
            })),
        );
        if compact {
            actions = actions.w_full().justify_end();
        }

        let mut card = div()
            .w_full()
            // The slot owns 4 px above and below. `h_full()` would resolve
            // against the slot's full height before those paddings and make
            // the card overflow downward, producing asymmetric whitespace.
            .h(px(row_h - 8.0))
            .flex()
            .px(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(border_color)
            .bg(ShellDeckColors::bg_sidebar());
        if compact {
            // Text line boxes carry more invisible space above their glyphs
            // than the fixed-height buttons do below. Bias the centered group
            // upward by 2 px so the *visible* top/bottom whitespace matches.
            card = card.flex_col().justify_center().gap(px(3.0)).pb(px(4.0));
        } else {
            card = card.items_center().gap(px(10.0));
        }

        div()
            .w_full()
            .h(px(row_h))
            .py(px(4.0))
            .child(card.child(identity).child(actions))
    }

    /// Navigation row of the User-mode home: the brand, the tabs, then the
    /// account. Tabs are quiet text buttons; the current one is tinted with
    /// the brand colour.
    pub(super) fn render_user_home_tab_bar(
        &self,
        account: AnyElement,
        roomy: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab = |label: String,
                   icon: &'static str,
                   target: UserHomeTab,
                   this_tab: UserHomeTab,
                   cx: &mut Context<Self>| {
            let active = this_tab == target;
            let entity = cx.entity();
            let button = adabraka_ui::components::button::Button::new(
                ElementId::from(SharedString::from(format!("uh-tab-{target:?}"))),
                label,
            )
            .size(adabraka_ui::components::button::ButtonSize::Sm)
            .h(px(34.0))
            .px(px(11.0))
            .icon(IconSource::from(icon))
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    if target == UserHomeTab::Share {
                        this.open_personal_share(cx);
                        this.personal_share.read(cx).focus_in(window);
                        return;
                    }
                    this.user_home_tab = target;
                    cx.notify();
                });
            });
            if active {
                // The tinted variant gives the label and the icon the brand
                // colour on a light tint of it; a tab carries no outline.
                button
                    .variant(ButtonVariant::Ai)
                    .border_color(gpui::transparent_black())
            } else {
                button.variant(ButtonVariant::Ghost)
            }
        };
        let current = self.user_home_tab;
        let tabs = div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .items_center()
            .flex_wrap()
            .gap(px(2.0))
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
                t!("share.tab").to_string(),
                "upload",
                UserHomeTab::Share,
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
            ));
        let mut bar = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap(px(14.0))
            .px(px(18.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.7))
            .bg(ShellDeckColors::bg_sidebar());
        if roomy {
            // The brand opens the row where there is room for it; a narrower
            // window keeps the width for the tabs.
            bar = bar.child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(8.0))
                    .pr(px(6.0))
                    .child(crate::brand::brand_badge(24.0))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child("Inklura"),
                    ),
            );
        }
        bar.child(tabs).child(account)
    }

    pub(super) fn render_user_overview(
        &self,
        compact: bool,
        wide_banner: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .collect::<Vec<_>>();
        let open_requests = my_requests
            .iter()
            .filter(|issue| !issue.is_closed())
            .count();

        // Counters that open the matching tab: the number is the shortcut.
        let stat = |icon: &'static str,
                    value: usize,
                    label: String,
                    target: UserHomeTab,
                    cx: &mut Context<Self>| {
            let entity = cx.entity();
            home_panel()
                .id(ElementId::from(SharedString::from(format!(
                    "home-stat-{target:?}"
                ))))
                .flex()
                .flex_1()
                .min_w(px(200.0))
                .items_center()
                .gap(px(14.0))
                .px(px(18.0))
                .py(px(11.0))
                .cursor_pointer()
                .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.user_home_tab = target;
                        cx.notify();
                    });
                })
                .child(home_icon_tile(icon, ShellDeckColors::primary(), true))
                .child(
                    div()
                        .text_size(px(24.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(value.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(px(14.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(label),
                )
                .child(lucide_icon(
                    "chevron-right",
                    16.0,
                    ShellDeckColors::text_muted(),
                ))
        };

        let entity = cx.entity();
        let capture_share = Button::new(
            "home-capture-share",
            t!("palette.capture_share").to_string(),
        )
        .variant(ButtonVariant::Outline)
        .icon(IconSource::from("scan"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| this.capture_and_share(Box::new(|_| {}), cx));
        });
        let entity = cx.entity();
        let share_files = Button::new("home-share-files", t!("palette.share_files").to_string())
            .variant(ButtonVariant::Outline)
            .icon(IconSource::from("upload"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.share_files_now(cx));
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
                    let action_id =
                        ElementId::from(SharedString::from(format!("home-recent-request-{index}")));
                    let mut row = div()
                        .id(action_id.clone())
                        .flex()
                        .items_center()
                        .gap(px(14.0))
                        .mx(px(-8.0))
                        .px(px(8.0))
                        .py(px(11.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()));
                    if index > 0 {
                        // A rule between two requests, none under the last.
                        row = row
                            .border_t_1()
                            .border_color(ShellDeckColors::border().opacity(0.55));
                    }
                    let row = row
                        .child(home_icon_tile("mail", ShellDeckColors::primary(), false))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .min_w(px(0.0))
                                .flex_1()
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .overflow_hidden()
                                        .child(crate::external_content::external_title(
                                            &issue.title,
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(updated),
                                ),
                        )
                        .child(home_status_pill(&issue.status));
                    KeyboardAction::new(action_id, row, move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.user_home_tab = UserHomeTab::Requests;
                            this.select_issue(issue_id.clone(), cx);
                            cx.notify();
                        });
                    })
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
        let status_row =
            |icon: &'static str, label: String, value: String, color: Hsla, first: bool| {
                let mut row = div().flex().items_center().gap(px(14.0)).py(px(10.0));
                if !first {
                    row = row
                        .border_t_1()
                        .border_color(ShellDeckColors::border().opacity(0.55));
                }
                row.child(home_icon_tile(icon, color, false)).child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .flex_1()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
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

        // The banner is the street of inklura.fr (`ludique.rs`): the copy and
        // the main action on the sky to the left, the town on its stretch of
        // road to the right. Everything over the sky takes the scene's own
        // colours, never the palette's: the scene keeps its day or night
        // whatever the theme.
        let night = crate::ludique::street_is_night(cx);
        // Below the wide breakpoint the copy needs the larger share of the
        // banner and a smaller title to stay on one line.
        let roomy = wide_banner && !compact;
        let street = if roomy {
            crate::ludique::StreetLayout::Banner
        } else {
            crate::ludique::StreetLayout::BannerCompact
        };
        let entity = cx.entity();
        let new_request = KeyboardAction::new(
            "home-new-request",
            crate::ludique::chunky_button("home-new-request", crate::ludique::ChunkyKind::Primary)
                .rounded(px(10.0))
                .h(px(if roomy { 42.0 } else { 38.0 }))
                .px(px(if roomy { 20.0 } else { 16.0 }))
                .text_size(px(if roomy { 15.0 } else { 13.0 }))
                .child(
                    svg()
                        .path(lucide_path("plus"))
                        .size(px(16.0))
                        .text_color(crate::theme::StreetColors::paper()),
                )
                .child(t!("user.home.new_request").to_string()),
            move |_, cx| {
                entity.update(cx, |this, cx| this.open_new_request(cx));
            },
        );
        let hero_copy = div()
            .absolute()
            .top_0()
            .left_0()
            .bottom_0()
            // The copy may run a little into the fade of the town.
            .w(relative(street.copy_share() + 0.05))
            .flex()
            .flex_col()
            .justify_center()
            .items_start()
            .gap(px(if roomy { 8.0 } else { 6.0 }))
            .pl(px(if compact {
                18.0
            } else if roomy {
                30.0
            } else {
                24.0
            }))
            .pr(px(8.0))
            .child(
                crate::ludique::highlighted(
                    t!("user.home.title").to_string(),
                    if roomy {
                        30.0
                    } else if compact {
                        21.0
                    } else {
                        24.0
                    },
                    night,
                )
                .whitespace_nowrap(),
            )
            .child(
                div()
                    .max_w(px(350.0))
                    .text_size(px(if roomy { 15.0 } else { 13.0 }))
                    .text_color(crate::theme::StreetColors::ink_muted(night))
                    .child(t!("user.home.subtitle").to_string()),
            )
            // Room under the button for the edge it stands on.
            .child(div().pt(px(6.0)).pb(px(4.0)).child(new_request));
        let hero = Self::street_banner(
            "user-home",
            street,
            if roomy { 212.0 } else { 204.0 },
            hero_copy,
            cx,
        );

        let entity = cx.entity();
        let see_all = Button::new("home-recent-see-all", t!("user.home.see_all").to_string())
            .variant(ButtonVariant::Link)
            .size(ButtonSize::Sm)
            .h(px(26.0))
            .px(px(4.0))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.user_home_tab = UserHomeTab::Requests;
                    cx.notify();
                });
            });

        div()
            .id("user-overview-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .gap(px(14.0))
            .p(px(18.0))
            .child(hero)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(14.0))
                    .child(stat(
                        "globe",
                        sites,
                        t!("user.home.sites").to_string(),
                        UserHomeTab::Sites,
                        cx,
                    ))
                    .child(stat(
                        "inbox",
                        open_requests,
                        t!("user.home.open_requests_count").to_string(),
                        UserHomeTab::Requests,
                        cx,
                    )),
            )
            .child(
                // Below the wide breakpoint the buttons move under the title
                // as one row. The main action, a new request, is in the banner.
                home_panel()
                    .flex()
                    .map(|row| {
                        if wide_banner {
                            row.items_center().justify_between()
                        } else {
                            row.flex_col().items_start()
                        }
                    })
                    .gap(px(14.0))
                    .px(px(22.0))
                    .py(px(15.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("user.home.quick_actions").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("user.home.quick_actions_hint").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .when(wide_banner, |buttons| {
                                buttons
                                    .justify_end()
                                    .pl(px(22.0))
                                    .border_l_1()
                                    .border_color(ShellDeckColors::border().opacity(0.7))
                            })
                            .gap(px(10.0))
                            .child(capture_share)
                            .child(share_files),
                    ),
            )
            .children(self.render_home_share_card(cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(14.0))
                    .child(
                        home_panel()
                            .min_w(px(300.0))
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .px(px(22.0))
                            .pt(px(16.0))
                            .pb(px(12.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.0))
                                    .child(home_section_title(
                                        "tag",
                                        t!("user.home.recent_requests").to_string(),
                                    ))
                                    .child(see_all),
                            )
                            .child(recent_requests),
                    )
                    .child(
                        home_panel()
                            .min_w(px(260.0))
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .px(px(22.0))
                            .pt(px(16.0))
                            .pb(px(10.0))
                            .child(home_section_title(
                                "activity",
                                t!("user.home.workspace_status").to_string(),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(status_row(
                                        "shield",
                                        t!("user.home.account").to_string(),
                                        account_status,
                                        self.account_status.dot_color(),
                                        true,
                                    ))
                                    .child(status_row(
                                        "globe",
                                        t!("user.home.active_site").to_string(),
                                        active_site,
                                        ShellDeckColors::primary(),
                                        false,
                                    ))
                                    .child(status_row(
                                        "database",
                                        t!("user.home.directory").to_string(),
                                        directory_count,
                                        ShellDeckColors::success(),
                                        false,
                                    )),
                            ),
                    ),
            )
    }

    /// A rounded banner of the User pages: the street scene, and `copy` over
    /// its sky (an absolute layer the caller sizes).
    fn street_banner(
        id: &'static str,
        layout: crate::ludique::StreetLayout,
        height: f32,
        copy: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        // Clipping is rectangular: the road would show a square corner at the
        // bottom right of the rounded banner. A ring in the page colour paints
        // it away: its inner edge is the banner's own curve, and it is wide
        // enough to reach the corner. Only its bottom right quarter is drawn,
        // and the box is twice the outer radius, the largest a corner may be.
        let corner_ring = BANNER_RADIUS * 0.5;
        let corner_outer = BANNER_RADIUS - 1.0 + corner_ring;
        // No width of its own: the column stretches it. With a percentage
        // width the banner collapsed to its border in a 760 px window.
        div()
            .relative()
            .flex_shrink_0()
            .h(px(height))
            .overflow_hidden()
            .rounded(px(BANNER_RADIUS))
            .border_1()
            .border_color(ShellDeckColors::primary().opacity(0.18))
            .bg(user_home_page_bg())
            .child(crate::ludique::street_scene(id, layout, cx).rounded(px(BANNER_RADIUS - 1.0)))
            .child(
                div()
                    .absolute()
                    .right(px(-corner_ring))
                    .bottom(px(-corner_ring))
                    .size(px(corner_outer * 2.0))
                    .rounded_br(px(corner_outer))
                    .border_r(px(corner_ring))
                    .border_b(px(corner_ring))
                    .border_color(user_home_page_bg()),
            )
            .child(copy)
    }

    /// User-mode "Mes informations" tab: the page opens on a short banner of
    /// the street, then one panel per subject, each opened by a drawing of
    /// the street (`ludique::section_badge`). It surfaces the meaningful
    /// fields the `/whoami` payload returned plus the account bits and
    /// directory stats. Missing optional values are omitted rather than
    /// rendered as dashes, and a subject with nothing to show has no panel.
    /// Deliberately read-only so it can't accidentally mutate credentials.
    pub(super) fn render_user_infos_tab(
        &self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::ludique::{section_badge, SectionBadge, StreetLayout};

        let account = self.app_config.account.clone().unwrap_or_default();
        let server = self.account_base_url();
        let payload = self.site_directory.clone().unwrap_or_default();
        let role_tokens = user_role_tokens(&account);
        let whoami = self.last_whoami.clone().unwrap_or_default();

        // The rows of a panel: an icon tile, the label in small capitals, the
        // value in bold, and a rule between two rows.
        let fields = |rows: Vec<(String, String, &'static str)>| {
            let mut column = div().flex().flex_col();
            for (index, (label, value, icon)) in rows.into_iter().enumerate() {
                let mut row = div().flex().items_center().gap(px(14.0)).py(px(10.0));
                if index > 0 {
                    row = row
                        .border_t_1()
                        .border_color(ShellDeckColors::border().opacity(0.55));
                }
                column = column.child(
                    row.child(home_icon_tile(icon, ShellDeckColors::primary(), false))
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
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(value),
                                ),
                        ),
                );
            }
            column
        };

        // A panel: the drawing and the title, then the body. Panels flow two
        // per row on a wide window (one when narrow) and share the height of
        // their row.
        let section = |title: String, badge: SectionBadge, body: gpui::Div| {
            home_panel()
                .border_color(ShellDeckColors::text_primary().opacity(0.26))
                .flex()
                .flex_col()
                .flex_1()
                .flex_basis(px(480.0))
                .min_w(px(300.0))
                .gap(px(4.0))
                .px(px(22.0))
                .pt(px(16.0))
                .pb(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .pb(px(4.0))
                        .child(section_badge(badge, 50.0))
                        .child(
                            div()
                                .text_size(px(19.0))
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(title),
                        ),
                )
                .child(body)
        };

        // Account: identity + Manage server.
        let mut account_rows = Vec::new();
        if let Some(name) = nonempty_text(&account.display_name()) {
            account_rows.push((t!("user.infos.field.name").to_string(), name, "user"));
        }
        if let Some(email) = nonempty_text(&account.email) {
            account_rows.push((t!("user.infos.field.email").to_string(), email, "mail"));
        }
        if let Some(server) = nonempty_text(&server) {
            account_rows.push((t!("user.infos.field.server").to_string(), server, "globe"));
        }

        // Scope: tenant + sites the server exposed to us.
        let tenant_name = payload
            .sites
            .first()
            .map(|s| s.tenant_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_default();
        let sites_count = payload.sites.len();
        let mut scope_rows = Vec::new();
        if let Some(tenant_name) = nonempty_text(&tenant_name) {
            scope_rows.push((
                t!("user.infos.field.tenant").to_string(),
                tenant_name,
                "users",
            ));
        }
        let sites_count_label = if sites_count == 1 {
            t!("user.infos.field.sites_count.one").to_string()
        } else {
            t!("user.infos.field.sites_count.many", count = sites_count).to_string()
        };
        scope_rows.push((
            t!("user.infos.field.sites_available", count = sites_count).to_string(),
            sites_count_label,
            "globe",
        ));

        // Session: only values actually returned by whoami. Empty optional
        // timestamps must not turn into rows whose value is just a dash.
        let mut session_rows = Vec::new();
        if let Some(device) = whoami.label.as_deref().and_then(nonempty_text) {
            session_rows.push((t!("user.infos.field.device").to_string(), device, "laptop"));
        }
        if let Some(created_at) = whoami.created_at.as_deref().and_then(nonempty_text) {
            session_rows.push((
                t!("user.infos.field.since").to_string(),
                crate::i18n::local_timestamp(&created_at),
                "calendar",
            ));
        }
        if let Some(last_seen_at) = whoami.last_seen_at.as_deref().and_then(nonempty_text) {
            session_rows.push((
                t!("user.infos.field.last_seen").to_string(),
                crate::i18n::local_timestamp(&last_seen_at),
                "clock",
            ));
        }

        // Access: one human label per entry in the CM role bag, including
        // custom roles. Explicit capability flags supply one coherent fallback
        // only for legacy tokens whose bag is absent; they are never merged
        // into a non-empty bag. See `.agents/roles.md`.
        let mut roles_body = div().flex().flex_wrap().gap(px(8.0)).py(px(10.0));
        for role in &role_tokens {
            roles_body = roles_body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(7.0))
                    .rounded(px(9.0))
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::primary())
                    .child(lucide_icon("shield", 15.0, ShellDeckColors::primary()))
                    .child(user_role_label(role)),
            );
        }

        let night = crate::ludique::street_is_night(cx);
        let head_copy = div()
            .absolute()
            .top_0()
            .left_0()
            .bottom_0()
            // The title may run a little into the fade of the town.
            .w(relative(StreetLayout::PageHead.copy_share() + 0.05))
            .flex()
            .items_center()
            .pl(px(if compact { 18.0 } else { 30.0 }))
            .child(
                crate::ludique::highlighted(
                    t!("user.tabs.infos").to_string(),
                    if compact { 20.0 } else { 28.0 },
                    night,
                )
                .whitespace_nowrap(),
            );
        let head = Self::street_banner(
            "user-infos",
            StreetLayout::PageHead,
            if compact { 124.0 } else { 140.0 },
            head_copy,
            cx,
        );

        let mut panels = div().flex().flex_wrap().gap(px(14.0)).child(section(
            t!("user.infos.section.account").to_string(),
            SectionBadge::Account,
            fields(account_rows),
        ));
        panels = panels.child(section(
            t!("user.infos.section.scope").to_string(),
            SectionBadge::Shop,
            fields(scope_rows),
        ));
        if !session_rows.is_empty() {
            panels = panels.child(section(
                t!("user.infos.section.session").to_string(),
                SectionBadge::Laptop,
                fields(session_rows),
            ));
        }
        if !role_tokens.is_empty() {
            panels = panels.child(section(
                t!("user.infos.section.roles").to_string(),
                SectionBadge::Shield,
                roles_body,
            ));
        }

        div()
            .id("user-infos-tab")
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(18.0))
            .child(head)
            .child(panels)
    }

    /// User mode: a manage-centric home — account header + "Mes sites" list with
    /// per-site selection plus public-site and Manage deep links.
    /// Pre-login welcome landing — intercepts the render whenever the user
    /// is not signed in (there is no guest path).
    ///
    /// The screen is the street of inklura.fr (`ludique.rs`): the sky fills the
    /// window, the town stands on the road at the bottom, and the copy sits on
    /// the sky above it. One product promise and two actions (sign in, create
    /// an account): the installed app is an authentication surface, not a
    /// second marketing page, so broader Inklura claims stay on the website.
    ///
    /// Everything drawn over the sky takes its colours from `StreetColors`:
    /// the scene has its own day and night, which a click on the sun changes
    /// without touching the theme.
    ///
    /// `is_maximized` : comme le mode Utilisateur, l'écran de bienvenue est la
    /// couche opaque la plus basse — la barre d'état n'est pas montée ici.
    pub(super) fn render_welcome_screen(
        &self,
        is_maximized: bool,
        compact_height: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::ludique::{
            chunky_button, highlighted, sticker, street_is_night, street_scene, ChunkyKind,
            StreetLayout,
        };
        use crate::theme::StreetColors;

        let entity = cx.entity();
        let night = street_is_night(cx);
        let layout = if compact_height {
            StreetLayout::WelcomeCompact
        } else {
            StreetLayout::Welcome
        };
        let title_size = if compact_height { 24.0 } else { 32.0 };

        // Copy and actions, over the sky.
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .px(px(24.0))
            .child(sticker(t!("welcome.sticker").to_string()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap_x(px(title_size * 0.28))
                    .child(
                        div()
                            .text_size(px(title_size))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(StreetColors::ink(night))
                            .child(t!("welcome.title_lead").to_string()),
                    )
                    .child(highlighted(shelldeck_core::APP_NAME, title_size, night)),
            )
            .child(
                div()
                    .max_w(px(470.0))
                    .text_center()
                    .text_size(px(13.0))
                    .text_color(StreetColors::ink_muted(night))
                    .child(t!("welcome.tagline").to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap(px(12.0))
                    .mt(px(6.0))
                    // Room for the hard edge the buttons stand on.
                    .pb(px(6.0))
                    .child(
                        // Primary action: funnels to the existing LoginForm modal.
                        chunky_button("welcome-sign-in", ChunkyKind::Primary)
                            .h(px(44.0))
                            .px(px(22.0))
                            .text_size(px(14.0))
                            .child(
                                svg()
                                    .path(lucide_path("external-link"))
                                    .size(px(14.0))
                                    .text_color(StreetColors::paper()),
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
                        // Secondary action: opens Manage signup in the browser.
                        chunky_button("welcome-signup", ChunkyKind::Paper)
                            .h(px(44.0))
                            .px(px(20.0))
                            .text_size(px(13.0))
                            .child(t!("welcome.create_account").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.open_signup(cx));
                                }
                            }),
                    ),
            );

        // "Réalisé par WD29": same shape as the Settings > About signature,
        // written on the pavement in front of the road.
        const LOGO_H: f32 = 18.0;
        const FOOTER_H: f32 = 40.0;
        let made_by = round_window_bottom(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(FOOTER_H))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .bg(StreetColors::pavement(night))
                .text_color(StreetColors::line().opacity(0.72)),
            is_maximized,
        )
        .child(
            div()
                .text_size(px(11.0))
                .line_height(px(LOGO_H))
                .child(t!("settings.about.made_by").to_string()),
        )
        .child(
            svg()
                .path("images/wd29-logo.svg")
                .w(px(50.0))
                .h(px(LOGO_H))
                .flex_shrink_0()
                .text_color(StreetColors::line().opacity(0.72)),
        );

        // The copy is centred in the sky left above the buildings. At the
        // 400 px application minimum the same block scrolls instead, so
        // neither action disappears behind the town.
        let street_h = FOOTER_H + layout.road_height() + layout.town_height() * 0.66;
        let content = if compact_height {
            scrollable_vertical(
                div()
                    .id("welcome-body")
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .pt(px(22.0))
                    .pb(px(street_h))
                    .child(hero),
            )
            .into_any_element()
        } else {
            div()
                .id("welcome-body")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .pb(px(street_h))
                .child(hero)
                .into_any_element()
        };

        round_window_bottom(
            div()
                .relative()
                .size_full()
                .bg(StreetColors::sky_top(night))
                .overflow_hidden(),
            is_maximized,
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom(px(FOOTER_H))
                .child(street_scene("welcome", layout, cx)),
        )
        .child(made_by)
        .child(div().relative().size_full().child(content))
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
        let payload = self.site_directory.as_ref();
        let total_sites = payload.map_or(0, |directory| directory.sites.len());
        let role_tokens = user_role_tokens(&account);
        let primary_role = primary_user_role(&role_tokens)
            .filter(|role| *role != "user")
            .map(user_role_label);
        let compact_user_home = user_home_uses_compact_flow(
            window.viewport_size().width.to_f64() as f32,
            self.ui_font_size,
        );
        let wide_banner = user_home_uses_wide_banner(
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
            .filter_map(|key| {
                payload.and_then(|directory| {
                    directory
                        .areas
                        .iter()
                        .find(|area| area.key == *key)
                        .cloned()
                })
            })
            .collect();

        // Account strip: identity and the two account actions share the tab
        // row instead of a full-width card above it, so every User tab keeps
        // its height for content. The e-mail stays in Mes informations.
        let header = {
            let avatar = div()
                .size(px(30.0))
                .flex_shrink_0()
                .rounded_full()
                .bg(ShellDeckColors::primary().opacity(0.16))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::primary())
                .child(account.initial());
            let mut identity = div()
                .id("uh-account-identity")
                .flex()
                .items_center()
                .gap(px(8.0))
                .min_w(px(0.0))
                .child(avatar);
            if !compact_user_home {
                identity = identity.child(
                    div()
                        .max_w(px(220.0))
                        .truncate()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(account.display_name()),
                );
                // The strip consumes the same role presentation as Mes
                // informations. A contradictory legacy payload can therefore
                // never show two different access levels.
                // It is the first thing to go when the row gets tight.
                if let Some(role_label) = primary_role.clone().filter(|_| wide_banner) {
                    identity = identity.child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary().opacity(0.10))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::primary())
                            .child(lucide_icon("shield", 10.0, ShellDeckColors::primary()))
                            .child(role_label),
                    );
                }
            }
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(px(10.0))
                // A rule sets the account apart from the tabs.
                .pl(px(14.0))
                .border_l_1()
                .border_color(ShellDeckColors::border().opacity(0.7))
                .child(identity)
                .child(
                    Button::new("uh-open-manage", t!("user.open_manage").to_string())
                        .size(ButtonSize::Md)
                        .h(px(34.0))
                        .icon(IconSource::from("external-link"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.open_manage_area("/manage".to_string(), cx);
                        })),
                )
                .child(
                    IconButton::new("refresh-cw")
                        .variant(ButtonVariant::Ghost)
                        .size(gpui::px(30.0))
                        .icon_size(gpui::px(14.0))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cloud_sync_now(cx);
                        })),
                )
                .into_any_element()
        };

        // Sites: filter by search, sort (conn-bearing first, then alpha), and
        // split into (active-card, others-for-virt-list) once per render.
        let (active_site, others_sites) = self.partition_user_sites(cx);
        let others_sites = Rc::new(others_sites);
        let others_count = others_sites.len();
        let site_search_query = self
            .user_sites_search_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        let empty_state = user_sites_empty_state(
            total_sites,
            usize::from(active_site.is_some()) + others_count,
            &site_search_query,
        );

        let mut list = div()
            .id("user-home-sites")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(8.0))
            .px(px(16.0));

        if let Some(empty_state) = empty_state {
            let filtered = empty_state == UserSitesEmptyState::Filtered;
            let title = if filtered {
                t!(
                    "user.sites.empty.filtered.title",
                    query = site_search_query.as_str()
                )
                .to_string()
            } else {
                t!("user.sites.empty.title").to_string()
            };
            let hint = if filtered {
                t!("user.sites.empty.filtered.hint").to_string()
            } else {
                t!("user.sites.empty.hint").to_string()
            };
            // A piece of the street instead of a bare icon: a pin waiting for
            // its first site, or the balloon when the search finds nothing.
            let empty_icon = crate::ludique::vignette(
                "uh-sites-empty",
                if filtered {
                    crate::ludique::Vignette::Balloon
                } else {
                    crate::ludique::Vignette::Pin
                },
                cx,
            );
            let mut empty_card = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.0))
                .p(px(28.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_sidebar())
                .child(empty_icon)
                .child(
                    div()
                        .max_w(px(440.0))
                        .text_center()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(title),
                )
                .child(
                    div()
                        .max_w(px(440.0))
                        .text_center()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(hint),
                );
            if filtered {
                let search_state = self.user_sites_search_state.clone();
                empty_card = empty_card.child(
                    Button::new(
                        "uh-empty-clear-search",
                        t!("user.sites.empty.filtered.clear").to_string(),
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("x"))
                    .mt(px(4.0))
                    .on_click(move |_, _, cx| {
                        search_state.update(cx, |state, cx| state.reset(cx));
                    }),
                );
            } else {
                // A genuinely empty account keeps the two actions that can
                // populate it; filtered emptiness never suggests unrelated
                // Manage or sync work.
                empty_card = empty_card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .mt(px(4.0))
                        .child(
                            Button::new("uh-empty-open-manage", t!("user.open_manage").to_string())
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("external-link"))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.open_manage_area("/manage".to_string(), cx);
                                })),
                        )
                        .child(
                            Button::new("uh-empty-sync", t!("user.sync").to_string())
                                .variant(ButtonVariant::Outline)
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("refresh-cw"))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.cloud_sync_now(cx);
                                })),
                        ),
                );
            }
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
            const MIN_LIST_H: f32 = 120.0;
            let site_row_h = if compact_user_home {
                SITE_ROW_H_COMPACT
            } else {
                SITE_ROW_H
            };
            let visible_sites = Rc::clone(&others_sites);
            list = list.child(
                uniform_list(
                    "user-home-sites-virt",
                    others_count,
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        let mut items: Vec<AnyElement> = Vec::new();
                        for i in range {
                            if let Some(site) = visible_sites.get(i) {
                                items.push(
                                    this.render_compact_site_row(site, compact_user_home, cx)
                                        .into_any_element(),
                                );
                            }
                        }
                        items
                    }),
                )
                .w_full()
                .flex_1()
                .min_h(px(MIN_LIST_H.min(others_count as f32 * site_row_h))),
            );
        }

        // The account card and local navigation are shell chrome. Operational
        // content owns the remaining height; in Sites, the section heading is
        // fixed too and only the virtualised directory scrolls.
        let tab = self.user_home_tab;
        let tab_bar = self.render_user_home_tab_bar(header, wide_banner, cx);
        let tab_content = match tab {
            UserHomeTab::Share => div()
                .flex_1()
                .min_h(px(0.0))
                .child(self.personal_share.clone())
                .into_any_element(),
            UserHomeTab::Home => div()
                .flex_1()
                .min_h(px(0.0))
                .child(scrollable_vertical(div().pb(px(24.0)).child(
                    self.render_user_overview(compact_user_home, wide_banner, cx),
                )))
                .into_any_element(),
            UserHomeTab::Sites => {
                // This heading stays outside the long list so the current
                // page and its live filter remain available mid-scroll.
                let mut row = div()
                    .flex()
                    .flex_shrink_0()
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
                            .child(lucide_icon("globe", 16.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("user.sites.title").to_string()),
                            ),
                    );
                if total_sites > 5 {
                    let entity = cx.entity();
                    row = row.child(
                        div().w(px(260.0)).child(
                            Input::new(&self.user_sites_search_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("user.sites.search").to_string())
                                .prefix(lucide_icon("search", 12.0, ShellDeckColors::text_muted()))
                                .on_change(move |_, cx| {
                                    entity.update(cx, |_, cx| cx.notify());
                                }),
                        ),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .pb(px(24.0))
                    .child(row)
                    .child(list)
                    .children(if self.has_monique() {
                        Some(self.render_monique_ask_card(cx))
                    } else {
                        None
                    })
                    .into_any_element()
            }
            UserHomeTab::Requests => div()
                .flex_1()
                .min_h(px(0.0))
                .child(scrollable_vertical(
                    div()
                        .pb(px(24.0))
                        .child(self.render_user_requests(compact_user_home, cx)),
                ))
                .into_any_element(),
            UserHomeTab::Infos => div()
                .flex_1()
                .min_h(px(0.0))
                .child(scrollable_vertical(
                    div()
                        .pb(px(24.0))
                        .child(self.render_user_infos_tab(compact_user_home, cx)),
                ))
                .into_any_element(),
        };

        round_window_bottom(
            div()
                .size_full()
                .flex()
                .flex_col()
                // One page colour for every tab of the User home.
                .bg(user_home_page_bg())
                .overflow_hidden(),
            is_maximized,
        )
        .child(
            div()
                .id("user-home-body")
                .size_full()
                .flex()
                .flex_col()
                .child(tab_bar)
                .child(tab_content),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        capitalize_first, home_status_tone, humanize_custom_role, managed_site_public_url,
        nonempty_text, primary_user_role, user_home_uses_compact_flow, user_home_uses_wide_banner,
        user_role_tokens, user_sites_empty_state, welcome_uses_compact_flow, HomeStatusTone,
        UserSitesEmptyState,
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

    // SDTEST-1795 — SDUC-440. The account header and dashboard banner enter
    // their compact compositions together, and the threshold follows scale.
    #[test]
    fn user_home_compact_breakpoint_tracks_ui_scale() {
        assert!(user_home_uses_compact_flow(600.0, 14.0));
        assert!(!user_home_uses_compact_flow(601.0, 14.0));
        assert!(!user_home_uses_compact_flow(700.0, 14.0));
        assert!(user_home_uses_compact_flow(1_200.0, 28.0));
        assert!(!user_home_uses_compact_flow(1_201.0, 28.0));
    }

    // SDTEST-1913 — SDUC-440
    #[test]
    fn user_sites_empty_state_distinguishes_account_from_filtered_results() {
        assert_eq!(
            user_sites_empty_state(0, 0, ""),
            Some(UserSitesEmptyState::AccountEmpty)
        );
        assert_eq!(
            user_sites_empty_state(100, 0, "introuvable"),
            Some(UserSitesEmptyState::Filtered)
        );
        assert_eq!(user_sites_empty_state(100, 100, ""), None);
        assert_eq!(user_sites_empty_state(100, 1, "atelier"), None);
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

    // SDTEST-2909 — SDUC-440
    #[test]
    fn sdtest_2909_short_banner_only_where_the_subtitle_fits_on_one_line() {
        assert!(user_home_uses_wide_banner(1_100.0, 14.0));
        assert!(!user_home_uses_wide_banner(1_099.0, 14.0));
        assert!(!user_home_uses_wide_banner(860.0, 14.0));
        // The breakpoint is logical: a doubled UI scale needs twice the pixels.
        assert!(!user_home_uses_wide_banner(1_400.0, 28.0));
        assert!(user_home_uses_wide_banner(2_200.0, 28.0));
    }

    // SDTEST-3181 / SDUC-588. On the home page one status must catch the eye:
    // a request that waits for someone. Its label is written in lower case
    // for running text and starts with an accented letter in French.
    #[test]
    fn sdtest_3181_home_status_pills_single_out_what_waits() {
        assert_eq!(home_status_tone("open"), HomeStatusTone::Waiting);
        for status in [
            "triaging",
            "in_progress",
            "blocked",
            "done",
            "closed",
            "later",
        ] {
            assert_ne!(
                home_status_tone(status),
                HomeStatusTone::Waiting,
                "{status}"
            );
        }
        assert_eq!(home_status_tone("in_progress"), HomeStatusTone::Active);
        assert_eq!(home_status_tone("blocked"), HomeStatusTone::Blocked);
        assert_eq!(home_status_tone("done"), home_status_tone("closed"));
        assert_eq!(home_status_tone("later"), HomeStatusTone::Review);
        assert_eq!(capitalize_first("à traiter"), "À traiter");
        assert_eq!(capitalize_first("en cours"), "En cours");
        assert_eq!(capitalize_first(""), "");
    }
}
