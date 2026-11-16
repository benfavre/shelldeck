//! "Choisir un site" modal. An action that needs an active site (Ouvrir
//! Manage, a Manage area from the palette) opens it instead of a dead-end
//! toast. Picking a site makes it active and continues the action.

use super::*;
use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;

/// Open picker: the Manage area to continue to and its search field.
pub(super) struct SitePicker {
    area_path: String,
    search: Entity<InputState>,
    focused: bool,
}

/// Sites matching `query` (label, host or tenant, case-insensitive), sorted by
/// label so the list reads like a directory.
fn site_picker_matches<'a>(sites: &'a [ManagedSiteInfo], query: &str) -> Vec<&'a ManagedSiteInfo> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<(String, &ManagedSiteInfo)> = sites
        .iter()
        .filter(|s| {
            q.is_empty()
                || s.display_label().to_lowercase().contains(&q)
                || s.host.to_lowercase().contains(&q)
                || s.tenant_name.to_lowercase().contains(&q)
        })
        .map(|s| (s.display_label().to_lowercase(), s))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, s)| s).collect()
}

/// First letter of the site, for its tile.
fn site_initial(label: &str) -> String {
    label
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string())
}

impl Workspace {
    /// Ask for a site, then open `area_path` on it. Returns false when there
    /// is nothing to choose from (no directory or no site), so the caller can
    /// fall back to its own message.
    pub(super) fn open_site_picker(&mut self, area_path: String, cx: &mut Context<Self>) -> bool {
        if self
            .site_directory
            .as_ref()
            .is_none_or(|p| p.sites.is_empty())
        {
            return false;
        }
        self.site_menu_open = false;
        self.site_picker = Some(SitePicker {
            area_path,
            search: cx.new(InputState::new),
            focused: false,
        });
        cx.notify();
        true
    }

    fn close_site_picker(&mut self, cx: &mut Context<Self>) {
        self.site_picker = None;
        cx.notify();
    }

    fn pick_site_and_continue(&mut self, site: ManagedSiteInfo, cx: &mut Context<Self>) {
        let Some(picker) = self.site_picker.take() else {
            return;
        };
        self.select_site(Some(site.site_id.clone()), Some(site.display_label()), cx);
        self.open_manage_area(picker.area_path, cx);
    }

    pub(super) fn render_site_picker(
        &mut self,
        is_maximized: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.site_picker.as_mut()?;
        let search = picker.search.clone();
        if !picker.focused {
            picker.focused = true;
            window.focus(&search.read(cx).focus_handle(cx));
        }
        let query = search.read(cx).content().to_string();
        let sites = self
            .site_directory
            .as_ref()
            .map(|p| p.sites.clone())
            .unwrap_or_default();
        let matches: Vec<ManagedSiteInfo> = site_picker_matches(&sites, &query)
            .into_iter()
            .cloned()
            .collect();

        let entity = cx.entity();
        let mut list = div()
            .id("site-picker-list")
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(8.0))
            // Same sheet as the menus (`.agents/theming.md`), not the
            // dialog's default card grey.
            .bg(ShellDeckColors::popover_bg())
            .max_h(px(360.0))
            .overflow_y_scroll();
        if matches.is_empty() {
            list = list.child(
                div()
                    .py(px(24.0))
                    .flex()
                    .justify_center()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.sites.empty.filtered.title", query = query.trim()).to_string()),
            );
        }
        for (index, site) in matches.iter().enumerate() {
            let label = site.display_label();
            let brand = parse_brand_hex(&site.brand_color).unwrap_or(ShellDeckColors::primary());
            let detail = if site.host.is_empty() {
                site.tenant_name.clone()
            } else {
                site.host.clone()
            };
            let pick = site.clone();
            let entity = entity.clone();
            list = list.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "site-picker-{}",
                        site.site_id
                    ))))
                    .debug_selector(move || format!("site-picker-row-{index}"))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .on_click(move |_, _, cx| {
                        let pick = pick.clone();
                        entity.update(cx, |this, cx| this.pick_site_and_continue(pick, cx));
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .size(px(32.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(brand.opacity(0.14))
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(brand)
                            .child(site_initial(&label)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(label),
                            )
                            .when(!detail.is_empty(), |el| {
                                el.child(
                                    div()
                                        .truncate()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(detail),
                                )
                            }),
                    )
                    .child(div().flex_shrink_0().child(lucide_icon(
                        "external-link",
                        14.0,
                        ShellDeckColors::text_muted(),
                    ))),
            );
        }

        let first = matches.first().cloned();
        let enter_entity = entity.clone();
        let change_entity = entity.clone();
        let backdrop_entity = entity.clone();
        let cancel_entity = entity.clone();
        let escape_entity = entity;

        Some(
            div()
                .id("site-picker-root")
                .absolute()
                .inset_0()
                // Escape is bound to the search field's own action, which
                // runs before any key listener; intercept that action.
                .capture_action(move |_: &adabraka_ui::components::input::Escape, _, cx| {
                    escape_entity.update(cx, |this, cx| this.close_site_picker(cx));
                    cx.stop_propagation();
                })
                .child(
                    UiDialog::new()
                        .width(gpui::px(460.0))
                        .backdrop_radius(if is_maximized {
                            gpui::px(0.0)
                        } else {
                            use_theme().tokens.radius_xl
                        })
                        .on_backdrop_click(move |_, cx| {
                            backdrop_entity.update(cx, |this, cx| this.close_site_picker(cx));
                        })
                        .header(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(12.0))
                                .px(px(20.0))
                                .pt(px(18.0))
                                .pb(px(14.0))
                                .bg(ShellDeckColors::popover_bg())
                                .rounded_t(use_theme().tokens.radius_lg)
                                .border_b_1()
                                .border_color(ShellDeckColors::border())
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(4.0))
                                        .child(
                                            div()
                                                .text_size(px(16.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(ShellDeckColors::text_primary())
                                                .child(t!("site_picker.title").to_string()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .text_color(ShellDeckColors::text_muted())
                                                .child(t!("site_picker.hint").to_string()),
                                        ),
                                )
                                .child(
                                    Input::new(&search)
                                        .placeholder(t!("user.sites.search").to_string())
                                        .prefix(lucide_icon(
                                            "search",
                                            14.0,
                                            ShellDeckColors::text_muted(),
                                        ))
                                        .on_change(move |_, cx| {
                                            change_entity.update(cx, |_, cx| cx.notify());
                                        })
                                        .on_enter(move |_, cx| {
                                            if let Some(site) = first.clone() {
                                                enter_entity.update(cx, |this, cx| {
                                                    this.pick_site_and_continue(site, cx)
                                                });
                                            }
                                        }),
                                ),
                        )
                        .content(list)
                        .footer(
                            div()
                                .flex()
                                .justify_end()
                                .px(px(16.0))
                                .py(px(12.0))
                                .bg(ShellDeckColors::popover_bg())
                                .rounded_b(use_theme().tokens.radius_lg)
                                .border_t_1()
                                .border_color(ShellDeckColors::border())
                                .child(
                                    Button::new(
                                        "site-picker-cancel",
                                        t!("site_picker.cancel").to_string(),
                                    )
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .on_click(
                                        move |_, _, cx| {
                                            cancel_entity
                                                .update(cx, |this, cx| this.close_site_picker(cx));
                                        },
                                    ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{site_initial, site_picker_matches};
    use shelldeck_core::config::manage_sites::ManagedSiteInfo;

    fn site(id: &str, label: &str, host: &str, tenant: &str) -> ManagedSiteInfo {
        serde_json::from_str(&format!(
            r#"{{"site_id":"{id}","label":"{label}","host":"{host}","tenant_name":"{tenant}"}}"#
        ))
        .unwrap()
    }

    // SDTEST-3188
    #[test]
    fn sdtest_3188_site_picker_filters_by_label_host_or_tenant_and_sorts() {
        let sites = vec![
            site("1", "Zèbre Café", "zebre.fr", "Zèbre SARL"),
            site("2", "atelier Bois", "bois.example", "Durand"),
            site("3", "Marée", "", "Bois & Fils"),
        ];
        let ids = |q: &str| -> Vec<String> {
            site_picker_matches(&sites, q)
                .into_iter()
                .map(|s| s.site_id.clone())
                .collect()
        };
        assert_eq!(ids(""), ["2", "3", "1"]);
        assert_eq!(ids("  BOIS "), ["2", "3"]);
        assert_eq!(ids("zebre.fr"), ["1"]);
        assert!(ids("nothing").is_empty());
        assert_eq!(site_initial("  été"), "É");
        assert_eq!(site_initial("—"), "?");
    }
}
