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

    /// User Home card: storage in use and the latest links. Hidden until the
    /// gallery has loaded (accounts without Share simply never show it).
    pub(super) fn render_home_share_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::share_view::human_bytes;
        let summary = self.share_summary.as_ref()?;
        let usage = if summary.storage_bytes > 0 {
            t!(
                "user.home.share.usage",
                used = human_bytes(summary.used_bytes),
                total = human_bytes(summary.storage_bytes),
                files = summary.files
            )
            .to_string()
        } else {
            t!(
                "user.home.share.usage_unlimited",
                used = human_bytes(summary.used_bytes),
                files = summary.files
            )
            .to_string()
        };
        let entity = cx.entity();
        let open = Button::new("home-share-open", t!("user.home.share.open").to_string())
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("upload"))
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    this.open_personal_share(cx);
                    this.personal_share.read(cx).focus_in(window);
                });
            });
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(lucide_icon("upload", 15.0, ShellDeckColors::primary()))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("user.home.share.title").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(summary.plan_label.clone()),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "home-share-usage".into())
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(usage),
            );
        if summary.storage_bytes > 0 {
            body = body.child(
                adabraka_ui::components::progress::ProgressBar::new(summary.usage_ratio())
                    .size(adabraka_ui::components::progress::ProgressSize::Sm),
            );
        }
        if summary.recent.is_empty() {
            body = body.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.home.share.empty").to_string()),
            );
        }
        for (index, file) in summary.recent.iter().enumerate() {
            let link = file.viewer_url.clone();
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(4.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border().opacity(0.65))
                    .child(lucide_icon(
                        "paperclip",
                        13.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(crate::external_content::external_title(&file.filename)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(human_bytes(file.bytes)),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("home-share-copy-{index}")),
                            t!("share.copy").to_string(),
                        )
                        .debug_selector(move || format!("home-share-copy-{index}"))
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .on_click(move |_, _, cx| {
                            if shelldeck_core::config::share::share_url(&link) {
                                cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                            }
                        }),
                    ),
            );
        }
        body = body.child(div().flex().justify_end().pt(px(4.0)).child(open));
        Some(
            adabraka_ui::display::card::Card::new()
                .content(body)
                .into_any_element(),
        )
    }
}
