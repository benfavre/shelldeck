use super::*;

impl Workspace {
    pub(super) fn open_personal_share(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let manage = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.user_home_tab = UserHomeTab::Share;
        self.personal_share.update(cx, |view, cx| {
            view.configure(manage, token, cx);
            view.refresh(cx);
        });
        cx.notify();
    }

    /// Files handed over by the OS. Signed-out launches keep them until the
    /// login completes (an account change otherwise clears Share state).
    pub fn stage_external_share(&mut self, files: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        self.show_main_presentation(cx);
        if !self.signed_in() {
            self.pending_share_files = files;
            self.show_login_form(cx);
            self.show_toast(
                t!("share.external.sign_in").to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        if self.effective_mode() != AppMode::User && self.can_access_mode(AppMode::User) {
            self.set_mode(AppMode::User, cx);
        }
        self.open_personal_share(cx);
        self.personal_share
            .update(cx, |view, cx| view.stage_external(files, cx));
        cx.notify();
    }

    /// Re-applies a share intent that arrived before sign-in.
    pub(super) fn resume_pending_share(&mut self, cx: &mut Context<Self>) {
        if self.pending_share_files.is_empty() || !self.signed_in() {
            return;
        }
        let files = std::mem::take(&mut self.pending_share_files);
        self.stage_external_share(files, cx);
    }

    /// Switches to User → Share for a Share shortcut. Returns `false` when
    /// signed out (the login form is shown instead).
    fn enter_personal_share(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.signed_in() {
            self.show_main_presentation(cx);
            self.show_login_form(cx);
            return false;
        }
        if self.effective_mode() != AppMode::User && self.can_access_mode(AppMode::User) {
            self.set_mode(AppMode::User, cx);
        }
        self.open_personal_share(cx);
        true
    }

    /// "Capture & share": region capture first, then `on_ready` shows the
    /// window with the annotator. Signed-out runs `on_ready` immediately so
    /// the login form is visible.
    pub fn capture_and_share(
        &mut self,
        on_ready: crate::share_view::CaptureReady,
        cx: &mut Context<Self>,
    ) {
        if !self.enter_personal_share(cx) {
            on_ready(cx);
            return;
        }
        self.personal_share
            .update(cx, |view, cx| view.capture_external(on_ready, cx));
    }

    /// "Share files…": open Share and the native file picker.
    pub fn share_files_now(&mut self, cx: &mut Context<Self>) {
        if !self.enter_personal_share(cx) {
            return;
        }
        self.personal_share
            .update(cx, |view, cx| view.choose_files_external(cx));
    }

    /// Loads the Share gallery once per account in User mode so the Home card
    /// can show storage and recent links without opening the Share tab.
    pub(super) fn prefetch_personal_share(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in()
            || self.effective_mode() != AppMode::User
            || self.personal_share.read(cx).is_loaded_or_busy()
        {
            return;
        }
        let manage = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.personal_share.update(cx, |view, cx| {
            view.configure(manage, token, cx);
            view.refresh(cx);
        });
    }

    /// « Partages récents » on the User home: capture and add files, the
    /// latest links, then storage in use and « Tout voir » in the card's foot.
    /// Until the gallery has loaded (or for an account without Share) the
    /// card keeps its two actions and shows no list.
    pub(super) fn render_home_share_card(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::share_view::human_bytes;
        let rule = ShellDeckColors::border().opacity(0.55);

        let entity = cx.entity();
        let capture = Button::new(
            "home-capture-share",
            t!("user.home.share.capture").to_string(),
        )
        .variant(ButtonVariant::Outline)
        .icon(IconSource::from("scan"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| this.capture_and_share(Box::new(|_| {}), cx));
        });
        let entity = cx.entity();
        let add_files = Button::new(
            "home-share-files",
            t!("user.home.share.add_files").to_string(),
        )
        .variant(ButtonVariant::Outline)
        .icon(IconSource::from("upload"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| this.share_files_now(cx));
        });

        let mut card = div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(14.0))
            .border_1()
            .border_color(ShellDeckColors::border().opacity(0.55))
            .bg(ShellDeckColors::bg_sidebar())
            .shadow(vec![use_theme().tokens.shadow_sm].into())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .px(px(26.0))
                    .py(px(20.0))
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("user.home.share.recent").to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(10.0))
                            .child(capture)
                            .child(add_files),
                    ),
            );

        let Some(summary) = self.share_summary.as_ref() else {
            return card.into_any_element();
        };

        // The latest files as tiles: the picture itself for an image, its
        // icon and kind otherwise. A click opens the file's page; the link
        // button copies it.
        let mut list = div().px(px(26.0)).pb(px(22.0));
        if summary.recent.is_empty() {
            list = list.child(
                div()
                    .py(px(18.0))
                    .border_t_1()
                    .border_color(rule)
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.home.share.empty").to_string()),
            );
        } else {
            let mut grid = div().flex().flex_wrap().gap(px(16.0));
            for (index, file) in summary.recent.iter().enumerate() {
                grid = grid.child(Self::render_home_share_tile(index, file, cx));
            }
            list = list.child(grid);
        }
        card = card.child(list);

        let files = if summary.files == 1 {
            t!("user.home.share.files.one").to_string()
        } else {
            t!("user.home.share.files.many", count = summary.files).to_string()
        };
        let usage = if summary.storage_bytes > 0 {
            format!(
                "{} / {}",
                human_bytes(summary.used_bytes),
                human_bytes(summary.storage_bytes)
            )
        } else {
            human_bytes(summary.used_bytes)
        };
        let mut meter = div().flex().items_center().gap(px(12.0)).min_w(px(0.0));
        if summary.storage_bytes > 0 {
            meter = meter.child(
                div().w(px(80.0)).flex_shrink_0().child(
                    adabraka_ui::components::progress::ProgressBar::new(summary.usage_ratio())
                        .size(adabraka_ui::components::progress::ProgressSize::Sm),
                ),
            );
        }
        meter = meter.child(
            div()
                .debug_selector(|| "home-share-usage".into())
                .truncate()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted())
                .child(format!("{usage}  ·  {files}")),
        );
        let entity = cx.entity();
        let see_all = Button::new("home-share-open", t!("user.home.share.see_all").to_string())
            .variant(ButtonVariant::Link)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("arrow-right"))
            .icon_position(adabraka_ui::components::button::IconPosition::End)
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    this.open_personal_share(cx);
                    this.personal_share.read(cx).focus_in(window);
                });
            });
        card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .px(px(26.0))
                .py(px(16.0))
                .border_t_1()
                .border_color(rule)
                .bg(ShellDeckColors::bg_primary().opacity(0.6))
                .child(meter)
                .child(see_all),
        )
        .into_any_element()
    }

    /// One recent share on the home card: thumbnail on top, name and kind
    /// below, the copy-link button beside them.
    fn render_home_share_tile(
        index: usize,
        file: &crate::share_view::ShareSummaryFile,
        cx: &mut App,
    ) -> impl IntoElement {
        use crate::share_view::human_bytes;
        const RADIUS: f32 = 12.0;
        let open = file.viewer_url.clone();
        let link = file.viewer_url.clone();
        let meta = match super::user_home::share_file_kind(&file.filename) {
            Some(kind) => format!("{kind} · {}", human_bytes(file.bytes)),
            None => human_bytes(file.bytes),
        };
        let tooltip: SharedString = t!("user.home.share.copy_link").to_string().into();
        // The id of an IconButton comes from its icon: the tile's own id
        // keeps the buttons apart.
        let copy = div()
            .id(SharedString::from(format!("home-share-copy-{index}")))
            .debug_selector(move || format!("home-share-copy-{index}"))
            .flex_shrink_0()
            .child(
                IconButton::new("link")
                    .variant(ButtonVariant::Ghost)
                    .size(gpui::px(30.0))
                    .icon_size(gpui::px(15.0))
                    .tooltip(move |_, cx| {
                        cx.new(|_| WorkspaceTooltip {
                            label: tooltip.clone(),
                        })
                        .into()
                    })
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        if shelldeck_core::config::share::share_url(&link) {
                            cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                        }
                    }),
            );
        div()
            .id(SharedString::from(format!("home-share-tile-{index}")))
            .debug_selector(move || format!("home-share-tile-{index}"))
            .flex_1()
            .flex_basis(px(170.0))
            .min_w(px(150.0))
            .max_w(px(320.0))
            .flex()
            .flex_col()
            .rounded(px(RADIUS))
            .border_1()
            .border_color(ShellDeckColors::border().opacity(0.7))
            .bg(ShellDeckColors::bg_sidebar())
            .cursor_pointer()
            .hover(|el| el.border_color(ShellDeckColors::primary().opacity(0.5)))
            .on_click(move |_, _, _| {
                if shelldeck_core::config::share::share_url(&open) {
                    let _ = open::that_detached(open.clone());
                }
            })
            .child(
                div()
                    .h(px(116.0))
                    .w_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(crate::share_view::thumbnail(
                        crate::share_view::ThumbFile {
                            id: &file.id,
                            filename: &file.filename,
                            kind: &file.kind,
                            thumbnail: file.thumbnail_url.as_deref(),
                        },
                        24.0,
                        crate::share_view::Radius::Top(RADIUS - 1.0),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(12.0))
                    .pr(px(6.0))
                    .py(px(8.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border().opacity(0.5))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(crate::external_content::external_title(&file.filename)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(meta),
                            ),
                    )
                    .child(copy),
            )
    }
}
