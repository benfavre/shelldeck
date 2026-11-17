use super::*;
use crate::overlay::round_window_bottom;
use std::rc::Rc;

const WELCOME_CENTERED_MIN_LOGICAL_HEIGHT: f32 = 560.0;
const USER_HOME_COMPACT_MAX_LOGICAL_WIDTH: f32 = 600.0;

pub(super) fn welcome_uses_compact_flow(viewport_height: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_height / scale < WELCOME_CENTERED_MIN_LOGICAL_HEIGHT
}

pub(super) fn user_home_uses_compact_flow(viewport_width: f32, ui_font_size: f32) -> bool {
    let scale = crate::scale::scale_for_font_size(ui_font_size);
    viewport_width / scale <= USER_HOME_COMPACT_MAX_LOGICAL_WIDTH
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
/// brand colour, so the panels stand out on it. Opaque on purpose: a street
/// banner paints it again over its own square corner (see `street_banner`).
fn user_home_page_bg() -> Hsla {
    ShellDeckColors::bg_primary().blend(ShellDeckColors::primary().opacity(0.045))
}

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

/// Title of a home card: the label alone, bold, in the text colour.
fn home_card_title(label: String) -> Div {
    crate::user_page::section_title(label)
}

/// First name for the greeting: the first word of the account name, else the
/// local part of the e-mail address.
fn greeting_name(account: &cloud_account::AccountInfo) -> String {
    account
        .name
        .split_whitespace()
        .next()
        .map(str::to_string)
        .unwrap_or_else(|| account.display_name())
}

/// A sentence around one of its words: the part before it and the part after.
/// Without the word, the whole sentence comes before.
fn split_around<'a>(text: &'a str, word: &str) -> (&'a str, &'a str) {
    text.split_once(word).unwrap_or((text, ""))
}

/// Requests that wait for someone: the count beside « Demandes récentes ».
fn home_waiting_count<'a>(statuses: impl Iterator<Item = &'a str>) -> usize {
    statuses
        .filter(|status| home_status_tone(status) == HomeStatusTone::Waiting)
        .count()
}

pub(super) use crate::share_view::file_kind as share_file_kind;

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

/// Colour of the dot in front of a request, by what it asks of the reader:
/// a request that waits for someone is the one in the warning colour.
fn home_status_dot(status: &str) -> Hsla {
    match home_status_tone(status) {
        HomeStatusTone::Waiting => ShellDeckColors::warning(),
        HomeStatusTone::Active => ShellDeckColors::primary(),
        HomeStatusTone::Review => ShellDeckColors::primary().opacity(0.45),
        HomeStatusTone::Blocked => ShellDeckColors::error(),
        HomeStatusTone::Settled => ShellDeckColors::text_muted().opacity(0.5),
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
            .px(px(16.0))
            .rounded(px(12.0))
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
    /// The left rail of User mode: one icon per tab, then « Ouvrir Manage »
    /// and the help at its foot. The account lives in the titlebar avatar.
    pub(super) fn render_user_rail(
        &self,
        is_maximized: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = self.user_home_tab;
        let rail_button =
            |id: SharedString, icon: &'static str, label: String, active: bool, outlined: bool| {
                let tooltip: SharedString = label.into();
                let mut button = div()
                    .id(ElementId::from(id))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(40.0))
                    .flex_shrink_0()
                    .rounded(px(10.0))
                    .cursor_pointer()
                    .tooltip(move |_, cx| {
                        cx.new(|_| WorkspaceTooltip {
                            label: tooltip.clone(),
                        })
                        .into()
                    });
                let icon_color = if active {
                    ShellDeckColors::primary()
                } else {
                    ShellDeckColors::text_muted()
                };
                button = if active {
                    button.bg(ShellDeckColors::primary().opacity(0.10))
                } else {
                    button.hover(|el| el.bg(ShellDeckColors::hover_bg()))
                };
                if outlined {
                    button = button
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_sidebar());
                }
                button.child(lucide_icon(icon, 17.0, icon_color))
            };

        let tabs = [
            (UserHomeTab::Home, "house", t!("user.tabs.home").to_string()),
            (
                UserHomeTab::Sites,
                "globe",
                t!("user.tabs.sites").to_string(),
            ),
            (UserHomeTab::Share, "folder-up", t!("share.tab").to_string()),
            (
                UserHomeTab::Requests,
                "ticket",
                t!("user.tabs.requests").to_string(),
            ),
            (
                UserHomeTab::Infos,
                "user",
                t!("user.tabs.infos").to_string(),
            ),
        ];
        let mut top = div().flex().flex_col().items_center().gap(px(6.0));
        for (target, icon, label) in tabs {
            let entity = cx.entity();
            top = top.child(
                rail_button(
                    format!("uh-tab-{target:?}").into(),
                    icon,
                    label,
                    current == target,
                    false,
                )
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
                }),
            );
        }

        let entity = cx.entity();
        let open_manage = rail_button(
            "uh-open-manage".into(),
            "external-link",
            t!("user.open_manage").to_string(),
            false,
            true,
        )
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.open_manage_area("/manage".to_string(), cx)
            });
        });
        let entity = cx.entity();
        let help = rail_button(
            "uh-help".into(),
            "circle-help",
            t!("user.rail.help").to_string(),
            false,
            false,
        )
        .on_click(move |_, window, cx| {
            entity.update(cx, |this, cx| {
                this.execute_menu_command(crate::menu_bar::MenuCommand::Documentation, window, cx)
            });
        });

        let mut rail = div().id("user-rail");
        // The rail reaches the bottom left corner of a floating window: the
        // opaque layer that touches a corner owns its radius.
        if !is_maximized {
            rail = rail.rounded_bl(use_theme().tokens.radius_xl);
        }
        rail.flex()
            .flex_col()
            .items_center()
            .justify_between()
            .flex_shrink_0()
            .w(px(60.0))
            .h_full()
            .py(px(14.0))
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border().opacity(0.7))
            .child(top)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .child(open_manage)
                    .child(help),
            )
    }

    /// User home (SDUC-588, second mock-up of 2026-11-16): a greeting with the
    /// title and the main action, then « Demandes récentes » beside « Mes
    /// sites », « Partages récents » under them, and the state of the Manage
    /// connection at the foot of the page. Palette colours only: the street
    /// stays on the welcome screen, "Mes informations" and the empty states.
    pub(super) fn render_user_overview(
        &self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let account = self.app_config.account.clone().unwrap_or_default();
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
        let waiting = home_waiting_count(my_requests.iter().map(|issue| issue.status.as_str()));

        // ── Head: greeting, title, main action ────────────────────────────
        let entity = cx.entity();
        let new_request = Button::new("home-new-request", t!("user.home.new_request").to_string())
            .size(ButtonSize::Lg)
            .icon(IconSource::from("plus"))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.open_new_request(cx));
            });
        let title_size = crate::user_page::title_size(compact);
        let title_text = |label: String| crate::user_page::title_text(label, compact);
        // « Bonjour Benjamin »: the name sits on a stroke of the marker pen,
        // like the brand word on the site. The sentence comes from the locale
        // and the name is cut out of it, so either language may place it.
        const NAME_SLOT: &str = "\u{1}";
        let name = greeting_name(&account);
        let greeting = t!("user.home.greeting", name = NAME_SLOT).to_string();
        let (before, after) = split_around(&greeting, NAME_SLOT);
        let marked_name = div()
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(title_size * 0.02))
                    .h(px(title_size * 0.12))
                    .rounded(px(2.0))
                    .bg(ShellDeckColors::warning().opacity(0.85)),
            )
            .child(title_text(name).relative());
        let mut title = div().flex().items_end().whitespace_nowrap();
        if !before.is_empty() {
            title = title.child(title_text(before.to_string()));
        }
        if greeting.contains(NAME_SLOT) {
            title = title.child(marked_name);
        }
        if !after.is_empty() {
            title = title.child(title_text(after.to_string()));
        }
        let head = crate::user_page::head(
            t!("user.home.eyebrow").to_string(),
            title,
            Some(new_request.into_any_element()),
        );

        // ── Demandes récentes ─────────────────────────────────────────────
        let mut requests_head = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(home_card_title(t!("user.home.recent_requests").to_string()));
        if waiting > 0 {
            let label = if waiting == 1 {
                t!("user.home.waiting_count.one").to_string()
            } else {
                t!("user.home.waiting_count.many", count = waiting).to_string()
            };
            requests_head = requests_head.child(
                div()
                    .flex_shrink_0()
                    .px(px(9.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::warning().opacity(0.18))
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(label),
            );
        }
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
                    let meta = format!(
                        "{} · {}",
                        capitalize_first(&crate::support_view::issue_status_label(&issue.status)),
                        rel_time(issue.updated_at)
                    );
                    let action_id =
                        ElementId::from(SharedString::from(format!("home-recent-request-{index}")));
                    let mut row = div()
                        .id(action_id.clone())
                        .flex()
                        .items_start()
                        .gap(px(14.0))
                        .mx(px(-8.0))
                        .px(px(8.0))
                        .py(px(14.0))
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
                        .child(
                            div()
                                .flex_shrink_0()
                                .mt(px(7.0))
                                .size(px(8.0))
                                .rounded_full()
                                .bg(home_status_dot(&issue.status)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .min_w(px(0.0))
                                .flex_1()
                                .child(
                                    div()
                                        .text_size(px(15.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .line_clamp(2)
                                        .child(crate::external_content::external_title(
                                            &issue.title,
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(meta),
                                ),
                        )
                        .child(div().flex_shrink_0().mt(px(2.0)).child(lucide_icon(
                            "chevron-right",
                            16.0,
                            ShellDeckColors::text_muted(),
                        )));
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
        let requests_card = home_panel()
            .min_w(px(300.0))
            .flex_grow()
            .flex_basis(relative(0.55))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(26.0))
            .pt(px(22.0))
            .pb(px(12.0))
            .child(requests_head)
            .child(recent_requests);

        // ── Mes sites ─────────────────────────────────────────────────────
        let entity = cx.entity();
        let browse = Button::new("home-sites-browse", t!("user.home.browse").to_string())
            .variant(ButtonVariant::Link)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("arrow-right"))
            .icon_position(adabraka_ui::components::button::IconPosition::End)
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.user_home_tab = UserHomeTab::Sites;
                    cx.notify();
                });
            });
        let sites_label = if sites == 1 {
            t!("user.home.sites_available.one").to_string()
        } else {
            t!("user.home.sites_available.many").to_string()
        };
        let active_site = self
            .app_config
            .cloud_sync
            .active_site_label
            .clone()
            .filter(|label| !label.trim().is_empty());
        let has_active_site = active_site.is_some();
        // The field opens the titlebar site switcher rather than a Select of
        // its own: one list of sites (pinned active, connection-bearing first,
        // capped) for one job, wherever it is opened from.
        let entity = cx.entity();
        let site_field = div()
            .id("home-active-site")
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .h(px(50.0))
            .px(px(14.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .cursor_pointer()
            .hover(|style| style.border_color(ShellDeckColors::primary().opacity(0.6)))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.open_site_switcher(cx));
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(14.0))
                    .text_color(if has_active_site {
                        ShellDeckColors::text_primary()
                    } else {
                        ShellDeckColors::text_muted()
                    })
                    .child(active_site.unwrap_or_else(|| t!("user.home.select_site").to_string())),
            )
            .child(lucide_icon(
                "chevron-down",
                16.0,
                ShellDeckColors::text_primary(),
            ));
        let sites_card = home_panel()
            .min_w(px(260.0))
            .flex_grow()
            .flex_basis(relative(0.4))
            .flex()
            .flex_col()
            .px(px(26.0))
            .pt(px(22.0))
            .pb(px(24.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(home_card_title(t!("user.tabs.sites").to_string()))
                    .child(browse),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(18.0))
                    .pt(px(18.0))
                    .pb(px(24.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border().opacity(0.55))
                    .child(
                        div()
                            .size(px(60.0))
                            .flex_shrink_0()
                            .rounded(px(12.0))
                            .bg(ShellDeckColors::primary().opacity(0.10))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(lucide_icon("globe", 20.0, ShellDeckColors::primary())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(34.0))
                                    .font_weight(FontWeight::EXTRA_BOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(sites.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(sites_label),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .pt(px(18.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.home.active_site").to_string()),
                    )
                    .child(site_field),
            );

        // ── Foot: refresh. The state of the Manage connection is the dot on
        // the titlebar avatar.
        let foot = div().flex().justify_end().child(
            Button::new("home-refresh", t!("user.home.refresh").to_string())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("refresh-cw"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.cloud_sync_now(cx);
                })),
        );

        crate::user_page::page(compact)
            .id("user-overview-scroll")
            .flex_1()
            .child(head)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(20.0))
                    .child(requests_card)
                    .child(sites_card),
            )
            .child(self.render_home_share_card(cx))
            .child(foot)
    }

    /// User-mode "Mes informations" tab: the head of every User screen, then
    /// one panel per subject, each opened by a drawing of the street
    /// (`ludique::section_badge`). It surfaces the meaningful
    /// fields the `/whoami` payload returned plus the account bits and
    /// directory stats. Missing optional values are omitted rather than
    /// rendered as dashes, and a subject with nothing to show has no panel.
    /// Deliberately read-only so it can't accidentally mutate credentials.
    pub(super) fn render_user_infos_tab(&self, compact: bool) -> impl IntoElement {
        use crate::ludique::{section_badge, SectionBadge};

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
                .px(px(26.0))
                .pt(px(16.0))
                .pb(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .pb(px(4.0))
                        .child(section_badge(badge, 44.0))
                        .child(crate::user_page::section_title(title)),
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

        // The head of every User screen; the street stays on the badges of
        // the panels below.
        let head = crate::user_page::head(
            t!("user.page.infos.eyebrow").to_string(),
            crate::user_page::title_text(t!("user.tabs.infos").to_string(), compact),
            None,
        );

        let mut panels = div()
            .flex()
            .flex_wrap()
            .gap(px(crate::user_page::PAGE_GAP))
            .child(section(
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

        crate::user_page::page(compact)
            .id("user-infos-tab")
            .child(head)
            .child(panels)
    }

    /// User mode: a manage-centric home — account header + "Mes sites" list with
    /// per-site selection plus public-site and Manage deep links.
    /// `is_maximized` sert uniquement aux deux coins bas : cette surface est la
    /// couche opaque la plus basse du mode Utilisateur, il n'y a pas de barre
    /// d'état sous elle pour les porter.
    pub(super) fn render_user_home(
        &self,
        is_maximized: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let payload = self.site_directory.as_ref();
        let total_sites = payload.map_or(0, |directory| directory.sites.len());
        let compact_user_home = user_home_uses_compact_flow(
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
            .px(px(crate::user_page::pad_x(compact_user_home)));

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
        let rail = (!self.app_config.general.user_rail_hidden)
            .then(|| self.render_user_rail(is_maximized, cx));
        let tab_content = match tab {
            UserHomeTab::Share => div()
                .flex_1()
                .min_h(px(0.0))
                .child(self.personal_share.clone())
                .into_any_element(),
            UserHomeTab::Home => div()
                .flex_1()
                .min_h(px(0.0))
                .child(scrollable_vertical(
                    div().child(self.render_user_overview(compact_user_home, cx)),
                ))
                .into_any_element(),
            UserHomeTab::Sites => {
                // This heading stays outside the long list so the current
                // page and its live filter remain available mid-scroll.
                let search = (total_sites > 5).then(|| {
                    let entity = cx.entity();
                    div()
                        .w(px(260.0))
                        .child(
                            Input::new(&self.user_sites_search_state)
                                .placeholder(t!("user.sites.search").to_string())
                                .prefix(lucide_icon("search", 14.0, ShellDeckColors::text_muted()))
                                .on_change(move |_, cx| {
                                    entity.update(cx, |_, cx| cx.notify());
                                }),
                        )
                        .into_any_element()
                });
                let row = crate::user_page::head(
                    t!("user.page.sites.eyebrow").to_string(),
                    crate::user_page::title_text(
                        t!("user.sites.title").to_string(),
                        compact_user_home,
                    ),
                    search,
                )
                .px(px(crate::user_page::pad_x(compact_user_home)))
                .pt(px(crate::user_page::PAGE_PAD_TOP))
                .pb(px(crate::user_page::PAGE_GAP));
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .pb(px(crate::user_page::PAGE_PAD_BOTTOM))
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
                    div().child(self.render_user_requests(compact_user_home, cx)),
                ))
                .into_any_element(),
            UserHomeTab::Infos => div()
                .flex_1()
                .min_h(px(0.0))
                .child(scrollable_vertical(
                    div().child(self.render_user_infos_tab(compact_user_home)),
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
                .children(rail)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .child(tab_content),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        capitalize_first, greeting_name, home_status_tone, home_waiting_count,
        humanize_custom_role, managed_site_public_url, nonempty_text, share_file_kind,
        split_around, user_home_uses_compact_flow, user_role_tokens, user_sites_empty_state,
        welcome_uses_compact_flow, HomeStatusTone, UserSitesEmptyState,
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

    // SDTEST-3183 / SDUC-588. The pieces of the home page that come from data:
    // the name cut out of the greeting in both languages, the first name of the
    // greeting, the count beside « Demandes récentes » and the kind of a
    // shared file.
    #[test]
    fn sdtest_3183_home_title_greeting_count_and_file_kind() {
        assert_eq!(split_around("Bonjour \u{1}", "\u{1}"), ("Bonjour ", ""));
        assert_eq!(split_around("Hello \u{1}!", "\u{1}"), ("Hello ", "!"));
        assert_eq!(split_around("Accueil", "\u{1}"), ("Accueil", ""));

        let named = AccountInfo {
            name: "Benjamin Favre".into(),
            email: "someone@example.org".into(),
            ..AccountInfo::default()
        };
        assert_eq!(greeting_name(&named), "Benjamin");
        let unnamed = AccountInfo {
            name: "  ".into(),
            email: "camille@example.org".into(),
            ..AccountInfo::default()
        };
        assert_eq!(greeting_name(&unnamed), "camille");

        let statuses = ["open", "closed", "open", "in_progress", "triaging"];
        assert_eq!(home_waiting_count(statuses.into_iter()), 2);
        assert_eq!(home_waiting_count(["done"].into_iter()), 0);

        assert_eq!(share_file_kind("image.png").as_deref(), Some("PNG"));
        assert_eq!(share_file_kind("archive.tar.gz").as_deref(), Some("GZ"));
        assert_eq!(share_file_kind("README"), None);
        assert_eq!(share_file_kind(".env"), None);
        assert_eq!(share_file_kind("notes.une extension"), None);
    }

    // SDTEST-3181 / SDUC-588. On the home page one status must catch the eye:
    // a request that waits for someone. Its label is written in lower case
    // for running text and starts with an accented letter in French.
    #[test]
    fn sdtest_3181_home_statuses_single_out_what_waits() {
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
