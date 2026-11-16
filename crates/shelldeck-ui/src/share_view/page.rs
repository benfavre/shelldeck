//! The Share page: head with the plan and storage, the intake card, then
//! « Mes fichiers » with its toolbar and the gallery. The whole page scrolls
//! as one virtualised list; the send bar and the transfer stay pinned below.

use super::gallery::{day_label, expiry, gallery_files, Expiry, FileFilter};
use super::*;
use crate::{icons::lucide_icon, scale::px, theme::ShellDeckColors};
use adabraka_ui::components::{
    button::{Button, ButtonSize, ButtonVariant},
    checkbox::Checkbox,
    icon_button::IconButton,
    icon_source::IconSource,
    input::Input,
    progress::{ProgressBar, ProgressSize},
};
use adabraka_ui::theme::use_theme;

// Single-line text on this page uses `line_clamp(1)`, not `truncate()`: a
// list measures its rows once at no width, and a no-wrap text keeps that
// first measure, cutting every name down to « … ».
const CARD_RADIUS: f32 = 12.0;
const GALLERY_PICTURE_H: f32 = 150.0;
const GALLERY_CARD_H: f32 = 292.0;
const GALLERY_GAP: f32 = 16.0;
const GALLERY_CARD_MIN_W: f32 = 230.0;
const ROW_H: f32 = 68.0;
const ROW_PICTURE_W: f32 = 92.0;
/// The User rail beside the page, in logical pixels.
const USER_RAIL_W: f32 = 60.0;

/// Cards per gallery row for the room the page leaves them.
pub(super) fn columns_for_width(width: f32) -> usize {
    (((width + GALLERY_GAP) / (GALLERY_CARD_MIN_W + GALLERY_GAP)).floor() as usize).clamp(1, 6)
}

fn ui_scale(window: &Window) -> f32 {
    (window.rem_size().to_f64() as f32 / crate::scale::REM_BASE).max(0.1)
}

fn tooltip(label: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label: SharedString = label.into();
    move |_, cx| {
        cx.new(|_| crate::workspace::WorkspaceTooltip {
            label: label.clone(),
        })
        .into()
    }
}

/// A panel of the page: the card surface of the User screens.
fn panel() -> Div {
    div()
        .rounded(px(14.0))
        .border_1()
        .border_color(ShellDeckColors::border().opacity(0.55))
        .bg(ShellDeckColors::bg_sidebar())
        .shadow(vec![use_theme().tokens.shadow_sm].into())
}

fn vertical_rule() -> Div {
    div()
        .w(px(1.0))
        .h(px(28.0))
        .flex_shrink_0()
        .bg(ShellDeckColors::border())
}

impl ShareView {
    /// « Inklura Share / Partage », the subtitle, then the plan, the storage
    /// in use, the account and « Actualiser ».
    fn render_head(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let title = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .min_w(px(0.0))
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Inklura Share"),
            )
            .child(crate::user_page::title_text(
                t!("share.tab").to_string(),
                compact,
            ))
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("share.subtitle").to_string()),
            );
        let mut stats = div().flex().flex_wrap().items_center().gap(px(16.0));
        if let Some(snapshot) = &self.snapshot {
            let ratio = if snapshot.plan.storage_bytes == 0 {
                0.0
            } else {
                (snapshot.usage.bytes as f64 / snapshot.plan.storage_bytes as f64).clamp(0.0, 1.0)
            };
            let percent = if ratio > 0.0 && ratio < 0.01 {
                "< 1 %".to_string()
            } else {
                format!("{} %", (ratio * 100.0).round() as u32)
            };
            stats =
                stats
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(4.0))
                            .rounded_full()
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::primary())
                            .child(snapshot.plan.label.clone()),
                    )
                    .child(vertical_rule())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(
                                        t!(
                                            "share.usage",
                                            used = human_bytes(snapshot.usage.bytes),
                                            total = human_bytes(snapshot.plan.storage_bytes)
                                        )
                                        .to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.0))
                                    .child(div().w(px(if compact { 140.0 } else { 220.0 })).child(
                                        ProgressBar::new(ratio as f32).size(ProgressSize::Sm),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(percent),
                                    ),
                            ),
                    );
            if !snapshot.owner.email.is_empty() {
                stats = stats.child(vertical_rule()).child(
                    div()
                        .text_size(px(14.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(snapshot.owner.email.clone()),
                );
            }
        }
        stats = stats.child(
            div()
                .id("share-refresh")
                .debug_selector(|| "share-refresh".into())
                .child(
                    IconButton::new("refresh-cw")
                        .variant(ButtonVariant::Outline)
                        .size(gpui::px(40.0))
                        .icon_size(gpui::px(17.0))
                        .disabled(self.busy)
                        .tooltip(tooltip(t!("share.refresh").to_string()))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
        );
        div()
            .flex()
            .flex_wrap()
            .items_start()
            .justify_between()
            .gap(px(16.0))
            .child(title)
            .child(stats)
            .into_any_element()
    }

    /// « Un fichier à partager ? »: the four ways in, the drop zone, the
    /// files waiting to be sent, then retention and the privacy note.
    fn render_intake(&mut self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .child(
                Button::new("share-pick", t!("share.files").to_string())
                    .icon(IconSource::from("upload"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.choose_files(cx))),
            )
            .child(
                Button::new("share-capture", t!("share.capture").to_string())
                    .icon(IconSource::from("scan"))
                    .variant(ButtonVariant::Outline)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.capture(cx))),
            )
            .child(
                Button::new("share-paste", t!("share.paste").to_string())
                    .icon(IconSource::from("clipboard-paste"))
                    .variant(ButtonVariant::Outline)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.paste(cx))),
            )
            .child(
                Button::new("share-pdf", "Inklura PDF")
                    .icon(IconSource::from("file-text"))
                    .variant(ButtonVariant::Outline)
                    .on_click(cx.listener(|this, _, _, cx| this.open_pdf(cx))),
            );
        let lead = div()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .flex_1()
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(
                        div()
                            .size(px(52.0))
                            .flex_shrink_0()
                            .rounded(px(12.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .child(lucide_icon("upload", 22.0, ShellDeckColors::primary())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_size(px(20.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("share.intake_title").to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("share.intake_hint").to_string()),
                            ),
                    ),
            )
            .child(actions);
        let mut top = div().flex().gap(px(24.0)).child(lead);
        if !compact {
            top = top.child(
                div()
                    .id("share-drop-zone")
                    .w(px(300.0))
                    .h(px(96.0))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_dashed()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .hover(|el| el.border_color(ShellDeckColors::primary().opacity(0.6)))
                    .on_click(cx.listener(|this, _, _, cx| this.choose_files(cx)))
                    .child(lucide_icon("upload", 22.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("share.drop").to_string()),
                    ),
            );
        }

        let mut card = panel()
            .id("share-intake")
            .debug_selector(|| "share-intake".into())
            .flex()
            .flex_col()
            .gap(px(16.0))
            .px(px(if compact { 16.0 } else { 24.0 }))
            .pt(px(22.0))
            .pb(px(18.0))
            .child(top);

        let mut messages = Vec::new();
        if let Some(snapshot) = &self.snapshot {
            if snapshot.plan.pending_france || !snapshot.plan.storage_ready {
                messages.push((
                    t!("share.error.france").to_string(),
                    ShellDeckColors::warning(),
                ));
            }
        }
        if let Some(error) = &self.error {
            messages.push((error.clone(), ShellDeckColors::error()));
        }
        if let Some(notice) = &self.notice {
            messages.push((notice.clone(), ShellDeckColors::success()));
        }
        if self.busy {
            messages.push((
                t!("share.working").to_string(),
                ShellDeckColors::text_muted(),
            ));
        }
        for (text, color) in messages {
            card = card.child(div().text_size(px(13.0)).text_color(color).child(text));
        }

        if !self.drafts.is_empty() {
            let mut drafts = div()
                .id("share-drafts")
                .debug_selector(|| "share-drafts".into())
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .max_h(px(100.0))
                .min_h(px(0.0))
                .flex_shrink_0()
                .overflow_y_scroll();
            for (index, draft) in self.drafts.iter().enumerate() {
                let mut chip = div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .max_w(px(320.0))
                    .min_w(px(0.0))
                    .pl(px(8.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(ShellDeckColors::border().opacity(0.7));
                chip = match &draft.preview {
                    Some(preview) => chip.child(
                        img(preview.image.clone())
                            .w(px(40.0))
                            .h(px(32.0))
                            .rounded(px(4.0))
                            .object_fit(ObjectFit::Cover),
                    ),
                    None => chip.child(lucide_icon(
                        super::file_icon(&draft.upload.filename),
                        16.0,
                        ShellDeckColors::text_muted(),
                    )),
                };
                drafts = drafts.child(
                    chip.child(
                        div()
                            .min_w(px(0.0))
                            .line_clamp(1)
                            .text_ellipsis()
                            .text_size(px(12.0))
                            .child(draft.upload.filename.clone()),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("share-draft-remove-{index}")),
                            "×",
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if index < this.drafts.len() {
                                this.drafts.remove(index);
                            }
                            cx.notify();
                        })),
                    ),
                );
            }
            card = card.child(drafts);
        }

        if self.snapshot.is_some() {
            let is_default = self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.default_retention_seconds == self.retention);
            let mut retention = div().flex().flex_wrap().items_center().gap(px(14.0)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(lucide_icon("clock", 16.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .child(t!("share.retention").to_string()),
                    ),
            );
            if let Some(select) = self.retention_select(cx) {
                retention = retention.child(
                    div()
                        .id("share-retention")
                        .debug_selector(|| "share-retention".into())
                        .w(px(160.0))
                        .child(select),
                );
            }
            retention = retention.child(
                div()
                    .id("share-default")
                    .debug_selector(|| "share-default".into())
                    .child(
                        Checkbox::new("share-default-check")
                            .label(t!("share.save_default").to_string())
                            .checked(is_default)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !is_default {
                                    this.save_default(cx);
                                }
                            })),
                    ),
            );
            card = card.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .pt(px(14.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border().opacity(0.55))
                    .child(retention)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(lucide_icon("lock", 14.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("share.private_note").to_string()),
                            ),
                    ),
            );
        }
        card.into_any_element()
    }

    /// « Mes fichiers »: search, family filters, order and grid/list.
    fn render_files_head(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let search = Input::new(&self.search)
            .placeholder(t!("share.search_files").to_string())
            .prefix(lucide_icon("search", 15.0, ShellDeckColors::text_muted()))
            .on_change(move |_, cx| view.update(cx, |_, cx| cx.notify()));
        let mut filters = div().flex().flex_wrap().items_center().gap(px(8.0));
        for filter in FileFilter::ALL {
            let mut button = Button::new(
                SharedString::from(format!("share-filter-{}", filter.key())),
                filter.label(),
            )
            .variant(ButtonVariant::Outline)
            .selected(self.filter == filter)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.filter = filter;
                cx.notify();
            }));
            if let Some(icon) = filter.icon() {
                button = button.icon(IconSource::from(icon));
            }
            filters = filters.child(button);
        }
        let mode = |id: &'static str, icon: &'static str, label: String, active: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .rounded(px(8.0))
                .when(active, |el| el.bg(ShellDeckColors::primary().opacity(0.12)))
                .child(
                    IconButton::new(icon)
                        .variant(ButtonVariant::Ghost)
                        .size(gpui::px(36.0))
                        .icon_size(gpui::px(17.0))
                        .tooltip(tooltip(label)),
                )
        };
        let modes = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .p(px(2.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .child(
                mode(
                    "share-view-grid",
                    "layout-grid",
                    t!("share.view_grid").to_string(),
                    !self.list_mode,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.list_mode = false;
                    cx.notify();
                })),
            )
            .child(
                mode(
                    "share-view-list",
                    "list",
                    t!("share.view_list").to_string(),
                    self.list_mode,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.list_mode = true;
                    cx.notify();
                })),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(if compact { 20.0 } else { 24.0 }))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("share.files_title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("share.files_hint").to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .w(px(if compact { 220.0 } else { 300.0 }))
                            .child(search),
                    )
                    .child(filters)
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("share-sort")
                            .debug_selector(|| "share-sort".into())
                            .w(px(170.0))
                            .child(self.sort_select.clone()),
                    )
                    .child(modes),
            )
            .into_any_element()
    }

    /// Date shared and expiry of a file, as shown under its name.
    fn file_dates(
        file: &shelldeck_core::config::share::ShareFile,
        today: chrono::NaiveDate,
    ) -> (String, AnyElement) {
        let mut meta = Vec::new();
        if let Some(kind) = super::file_kind(&file.filename) {
            meta.push(kind);
        }
        meta.push(human_bytes(file.bytes));
        if let Some(day) = day_label(&file.created_at, today) {
            meta.push(day);
        }
        let expiry = match expiry(file.expires_at.as_deref(), today) {
            Expiry::Never => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_color(ShellDeckColors::text_muted())
                .child(lucide_icon("infinity", 14.0, ShellDeckColors::text_muted()))
                .child(t!("share.expires.never").to_string()),
            Expiry::Later(label) => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_color(ShellDeckColors::text_muted())
                .child(lucide_icon("clock", 14.0, ShellDeckColors::text_muted()))
                .child(label),
            Expiry::Soon(label) => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .py(px(1.0))
                .rounded(px(6.0))
                .bg(ShellDeckColors::warning().opacity(0.14))
                .text_color(ShellDeckColors::warning())
                .child(lucide_icon("clock", 14.0, ShellDeckColors::warning()))
                .child(label),
        };
        (
            meta.join(" · "),
            expiry.text_size(px(12.0)).into_any_element(),
        )
    }

    /// « Copier le lien » and the « … » menu (open, rename, delete).
    fn file_actions(
        &self,
        file: &shelldeck_core::config::share::ShareFile,
        stretch: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let copy = file.viewer_url.clone();
        let id = file.id.clone();
        let menu_open = self.file_menu.as_deref() == Some(file.id.as_str());
        let mut more = div()
            .id(SharedString::from(format!("share-more-{}", file.id)))
            .debug_selector({
                let id = file.id.clone();
                move || format!("share-more-{id}")
            })
            .flex_shrink_0()
            .child(
                IconButton::new("ellipsis")
                    .variant(ButtonVariant::Outline)
                    .size(gpui::px(34.0))
                    .icon_size(gpui::px(16.0))
                    .tooltip(tooltip(t!("share.more").to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.file_menu = if this.file_menu.as_deref() == Some(id.as_str()) {
                            None
                        } else {
                            Some(id.clone())
                        };
                        cx.notify();
                    })),
            );
        if menu_open {
            more = more.child(deferred(
                anchored()
                    .snap_to_window_with_margin(gpui::px(8.0))
                    .anchor(Corner::TopRight)
                    .child(self.file_menu_panel(file, cx)),
            ));
        }
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div().when(stretch, |el| el.flex_1()).min_w(px(0.0)).child(
                    Button::new(
                        SharedString::from(format!("share-copy-{}", file.id)),
                        t!("share.copy").to_string(),
                    )
                    .debug_selector({
                        let id = file.id.clone();
                        move || format!("share-copy-{id}")
                    })
                    .when(stretch, |b| b.w_full())
                    .size(if stretch {
                        ButtonSize::Md
                    } else {
                        ButtonSize::Sm
                    })
                    .variant(ButtonVariant::Outline)
                    .icon(IconSource::from("link"))
                    .on_click(move |_, _, cx| {
                        if share_url(&copy) {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                        }
                    }),
                ),
            )
            .child(more)
    }

    fn file_menu_panel(
        &self,
        file: &shelldeck_core::config::share::ShareFile,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = |selector: String, icon: &'static str, label: String, color: Hsla| {
            div()
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .text_size(px(13.0))
                .text_color(color)
                .child(lucide_icon(icon, 15.0, color))
                .child(label)
        };
        let viewer = file.viewer_url.clone();
        let rename_id = file.id.clone();
        let original = file.filename.clone();
        let delete_id = file.id.clone();
        div()
            .id("share-file-menu")
            .occlude()
            .mt(px(6.0))
            .w(px(200.0))
            .p(px(6.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::popover_bg())
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.file_menu = None;
                cx.notify();
            }))
            .child(
                item(
                    format!("share-open-{}", file.id),
                    "external-link",
                    t!("share.open").to_string(),
                    ShellDeckColors::text_primary(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.file_menu = None;
                    if share_url(&viewer) {
                        let _ = open::that_detached(viewer.clone());
                    }
                    cx.notify();
                })),
            )
            .child(
                item(
                    format!("share-rename-{}", file.id),
                    "pencil",
                    t!("share.rename_title").to_string(),
                    ShellDeckColors::text_primary(),
                )
                .when(!self.busy, |el| {
                    el.on_click(cx.listener(move |this, _, window, cx| {
                        this.file_menu = None;
                        this.begin_rename(rename_id.clone(), original.clone(), window, cx);
                    }))
                }),
            )
            .child(
                item(
                    format!("share-delete-{}", file.id),
                    "trash-2",
                    t!("share.delete").to_string(),
                    ShellDeckColors::error(),
                )
                .when(!self.busy, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        this.file_menu = None;
                        this.confirm_delete = Some(delete_id.clone());
                        cx.notify();
                    }))
                }),
            )
            .into_any_element()
    }

    /// A gallery card: picture, name, kind · size · day, expiry, actions.
    fn render_card(
        &self,
        file: &shelldeck_core::config::share::ShareFile,
        today: chrono::NaiveDate,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open_picture = file.viewer_url.clone();
        let (meta, expiry) = Self::file_dates(file, today);
        let columns = self.columns.max(1) as f32;
        let card_w = ((self.room - GALLERY_GAP * (columns - 1.0)) / columns).max(120.0);
        div()
            .w(px(card_w))
            .flex_shrink_0()
            .h(px(GALLERY_CARD_H))
            .flex()
            .flex_col()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(ShellDeckColors::border().opacity(0.6))
            .bg(ShellDeckColors::bg_sidebar())
            .shadow(vec![use_theme().tokens.shadow_sm].into())
            .child(
                div()
                    .id(SharedString::from(format!("share-picture-{}", file.id)))
                    .h(px(GALLERY_PICTURE_H))
                    .w_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(move |_, _, _| {
                        if share_url(&open_picture) {
                            let _ = open::that_detached(open_picture.clone());
                        }
                    })
                    .child(super::thumbnail(
                        super::ThumbFile {
                            id: &file.id,
                            filename: &file.filename,
                            kind: &file.kind,
                            thumbnail: file.thumbnail_url.as_deref(),
                        },
                        26.0,
                        super::Radius::Top(CARD_RADIUS - 1.0),
                        cx,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .justify_between()
                    .px(px(14.0))
                    .pt(px(12.0))
                    .pb(px(12.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border().opacity(0.5))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .w_full()
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(crate::external_content::external_title(&file.filename)),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(meta),
                            )
                            .child(div().flex().child(expiry)),
                    )
                    .child(self.file_actions(file, true, cx)),
            )
            .into_any_element()
    }

    /// A row of the list view: picture at the left, the same information
    /// and actions in line.
    fn render_row(
        &self,
        file: &shelldeck_core::config::share::ShareFile,
        today: chrono::NaiveDate,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open_picture = file.viewer_url.clone();
        let (meta, expiry) = Self::file_dates(file, today);
        div()
            .w(px(self.room))
            .h(px(ROW_H))
            .flex()
            .items_center()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(ShellDeckColors::border().opacity(0.6))
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .id(SharedString::from(format!("share-picture-{}", file.id)))
                    .w(px(ROW_PICTURE_W))
                    .h_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(move |_, _, _| {
                        if share_url(&open_picture) {
                            let _ = open::that_detached(open_picture.clone());
                        }
                    })
                    .child(super::thumbnail(
                        super::ThumbFile {
                            id: &file.id,
                            filename: &file.filename,
                            kind: &file.kind,
                            thumbnail: file.thumbnail_url.as_deref(),
                        },
                        16.0,
                        super::Radius::Left(CARD_RADIUS - 1.0),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(14.0))
                    .border_l_1()
                    .border_color(ShellDeckColors::border().opacity(0.5))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(crate::external_content::external_title(&file.filename)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.0))
                                    .min_w(px(0.0))
                                    .child(
                                        // Short (kind · size · day): kept whole.
                                        div()
                                            .flex_shrink_0()
                                            .whitespace_nowrap()
                                            .text_size(px(12.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(meta),
                                    )
                                    .child(div().flex_shrink_0().child(expiry)),
                            ),
                    )
                    .child(self.file_actions(file, false, cx)),
            )
            .into_any_element()
    }

    /// One item of the page list: 0 is the top of the page, then the rows of
    /// the gallery, then its foot.
    fn render_page_item(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let compact = crate::user_page::is_compact(window);
        let pad = px(crate::user_page::pad_x(compact));
        let rows = self.gallery_rows();
        if index == 0 {
            // A list lays a row out at its content's width: rows ask for the
            // full width themselves.
            return div()
                .w_full()
                .px(pad)
                .pt(px(crate::user_page::PAGE_PAD_TOP))
                .pb(px(GALLERY_GAP + 4.0))
                .flex()
                .flex_col()
                .gap(px(crate::user_page::PAGE_GAP + 8.0))
                .child(self.render_head(compact, cx))
                .child(self.render_intake(compact, cx))
                .child(self.render_files_head(compact, cx))
                .into_any_element();
        }
        if index > rows {
            let mut foot = div()
                .w_full()
                .px(pad)
                .pb(px(crate::user_page::PAGE_PAD_BOTTOM))
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted());
            if self.shown.is_empty() && !self.busy {
                let empty = if self.search.read(cx).content().trim().is_empty()
                    && self.filter == FileFilter::All
                {
                    t!("share.empty")
                } else {
                    t!("share.empty_filtered")
                };
                foot = foot.child(div().py(px(24.0)).child(empty.to_string()));
            }
            if let Some(snapshot) = &self.snapshot {
                if snapshot.usage.files > snapshot.list_limit as u64 {
                    foot =
                        foot.child(t!("share.list_limit", count = snapshot.list_limit).to_string());
                }
            }
            return foot.into_any_element();
        }
        let today = chrono::Local::now().date_naive();
        let shown = self.shown.clone();
        // A fixed height: the list then lays the row out at that size, and
        // names keep their full width instead of a min-content measure.
        let mut line = div()
            .w_full()
            .h(px(if self.list_mode {
                ROW_H + 10.0
            } else {
                GALLERY_CARD_H + GALLERY_GAP
            }))
            .px(pad)
            .pb(px(if self.list_mode { 10.0 } else { GALLERY_GAP }))
            .flex()
            .gap(px(GALLERY_GAP));
        if self.list_mode {
            if let Some(file) = shown.get(index - 1) {
                line = line.child(self.render_row(file, today, cx));
            }
        } else {
            let start = (index - 1) * self.columns;
            for file in shown.iter().skip(start).take(self.columns) {
                line = line.child(self.render_card(file, today, cx));
            }
        }
        line.into_any_element()
    }

    fn gallery_rows(&self) -> usize {
        if self.list_mode {
            self.shown.len()
        } else {
            self.shown.len().div_ceil(self.columns.max(1))
        }
    }
}

impl Render for ShareView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(annotator) = &self.annotator {
            return div()
                .size_full()
                .child(annotator.clone())
                .into_any_element();
        }
        let compact = crate::user_page::is_compact(window);
        // What the gallery shows this frame; the list rows read it.
        let query = self.search.read(cx).content().to_string();
        self.shown = Arc::new(
            self.snapshot
                .as_ref()
                .map(|snapshot| gallery_files(&snapshot.files, &query, self.filter, self.sort))
                .unwrap_or_default(),
        );
        let page_width = match self.page_width.get() {
            width if width > 0.0 => width,
            // First frame: the window less the User rail.
            _ => window.viewport_size().width.to_f64() as f32 / ui_scale(window) - USER_RAIL_W,
        };
        self.room = (page_width - 2.0 * crate::user_page::pad_x(compact)).max(0.0);
        self.columns = columns_for_width(self.room);
        let count = 2 + self.gallery_rows();
        let old = self.page.item_count();
        if old == 0 {
            self.page.reset(count);
        } else if old != count {
            // Keep the top of the page (and the scroll position) when only
            // the gallery changes length.
            self.page.splice(1..old, count - 1);
        }

        let view = cx.entity();
        let measured = self.page_width.clone();
        let mut root = div()
            .id("personal-share")
            .relative()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .text_color(ShellDeckColors::text_primary())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.file_menu.is_some() {
                    this.file_menu = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key == "escape" && this.rename.is_some() {
                    this.rename = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key == "escape" && this.review_drafts {
                    this.review_drafts = false;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if this.rename.is_none()
                    && !this.search.read(cx).focus_handle(cx).is_focused(window)
                    && event.keystroke.key.eq_ignore_ascii_case("v")
                    && (modifiers.control || modifiers.platform)
                {
                    this.paste(cx);
                    cx.stop_propagation();
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_paths(paths.paths().to_vec(), cx)
            }))
            .child(
                canvas(
                    move |bounds, window, _| {
                        let width = bounds.size.width.to_f64() as f32 / ui_scale(window);
                        if (measured.get() - width).abs() > 0.5 {
                            measured.set(width);
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .debug_selector(|| "share-page".into())
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        list(self.page.clone(), move |index, window, cx| {
                            view.update(cx, |this, cx| this.render_page_item(index, window, cx))
                        })
                        .size_full(),
                    ),
            );

        // Pinned below the page: the transfer, or the files waiting to go.
        let bar = div()
            .flex_shrink_0()
            .px(px(crate::user_page::pad_x(compact)))
            .py(px(12.0))
            .border_t_1()
            .border_color(ShellDeckColors::border().opacity(0.6))
            .bg(ShellDeckColors::bg_sidebar());
        if self.upload_progress.is_some() {
            root = root.child(bar.child(self.progress_footer(cx)));
        } else if !self.drafts.is_empty() {
            root = root.child(
                bar.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(
                            Button::new(
                                "share-review",
                                t!("share.review", count = self.drafts.len()).to_string(),
                            )
                            .debug_selector(|| "share-review".into())
                            .variant(ButtonVariant::Outline)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review_drafts = true;
                                cx.notify();
                            })),
                        )
                        .child(
                            Button::new(
                                "share-send",
                                t!("share.send", count = self.drafts.len()).to_string(),
                            )
                            .flex_1()
                            .debug_selector(|| "share-send".into())
                            .icon(IconSource::from("upload"))
                            .disabled(
                                self.busy
                                    || self
                                        .snapshot
                                        .as_ref()
                                        .is_none_or(|snapshot| !snapshot.plan.storage_ready),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                        ),
                ),
            );
        }
        if self.review_drafts {
            root = root.child(self.review_dialog(window, cx));
        }
        if self.confirm_delete.is_some() {
            root = root.child(self.delete_dialog(cx));
        }
        if self.rename.is_some() {
            root = root.child(self.rename_dialog(cx));
        }
        root.into_any_element()
    }
}
