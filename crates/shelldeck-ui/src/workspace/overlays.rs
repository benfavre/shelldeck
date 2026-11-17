use super::*;
use crate::monolith::{animated_monolith, MonolithMotion};

impl Workspace {
    /// User-mode Monique card: a composer that stages a reviewed conversation
    /// turn, plus a read-only recent-process list.
    pub(super) fn render_monique_ask_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let input_display = if self.monique_ask_input.is_empty() {
            div()
                .text_color(ShellDeckColors::text_muted())
                .child(t!("user.monique.ask_placeholder").to_string())
        } else {
            div()
                .text_color(ShellDeckColors::text_primary())
                .child(self.monique_ask_input.clone())
        };

        let mut activity = div().flex().flex_col().gap(px(2.0)).mt(px(6.0));
        if let Some(processes) = &self.monique_processes {
            for process in processes.jobs.iter().take(10) {
                activity = activity.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .py(px(2.0))
                        .child(
                            div()
                                .flex_shrink_0()
                                .px(px(5.0))
                                .rounded(px(6.0))
                                .bg(ShellDeckColors::badge_bg())
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(process.status.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(if process.id.is_empty() {
                                    process.source.clone()
                                } else {
                                    process.id.clone()
                                }),
                        ),
                );
            }
        }

        div()
            .m(px(16.0))
            .p(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(lucide_icon("zap", 15.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("user.monique.ask_title").to_string()),
                    ),
            )
            .child(
                div()
                    .id("monique-ask-input")
                    .track_focus(&self.monique_ask_focus)
                    .on_key_down(cx.listener(|this, e: &KeyDownEvent, _w, cx| {
                        this.handle_monique_ask_key(e, cx)
                    }))
                    .w_full()
                    .min_h(px(56.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(13.0))
                    .cursor_text()
                    .child(input_display),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.monique.confirm_hint").to_string()),
                    )
                    .child(
                        div()
                            .id("monique-ask-send")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(7.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("send"))
                                    .size(px(12.0))
                                    .text_color(white()),
                            )
                            .child(t!("user.requests.send").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.submit_monique_ask(cx)
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .mt(px(4.0))
                    .child(t!("user.monique.recent_activity").to_string()),
            )
            .child(activity)
    }

    /// The loading screen after sign-in (SDUC-598): it stands under the
    /// signed-out title bar, on the welcome screen's page and foot, so signing
    /// in reads as one continuous screen.
    pub(super) fn render_post_login_splash(
        &self,
        splash: &PostLoginSplash,
        is_maximized: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use std::time::Duration;
        const BAR_W: f32 = 340.0;

        let dismissing = splash.dismissing;
        let mascot = animated_monolith(
            "post-login-mascot",
            112.0,
            MonolithMotion::FloatAndBreathe,
            cx,
        );

        let progress_bar = div()
            .relative()
            .w(px(BAR_W))
            .h(px(8.0))
            .overflow_hidden()
            .rounded_full()
            .bg(ShellDeckColors::primary().opacity(0.14))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(px(0.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary())
                    .with_animation(
                        "post-login-progress-bar",
                        Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS)),
                        move |el, delta| {
                            el.w(px(BAR_W * post_login_display_progress(dismissing, delta)))
                        },
                    ),
            );

        let progress_percentage = div()
            .min_w(px(40.0))
            .text_right()
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::primary())
            .with_animation(
                "post-login-progress-percentage",
                Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS)),
                move |el, delta| {
                    let percentage =
                        (post_login_display_progress(dismissing, delta) * 100.0).round() as u8;
                    el.child(format!("{percentage}%"))
                },
            );

        let card = div()
            .debug_selector(|| "post-login-card".into())
            .flex()
            .flex_col()
            .items_center()
            .w(px(BAR_W + 88.0))
            .px(px(44.0))
            .pt(px(32.0))
            .pb(px(36.0))
            .rounded(px(20.0))
            .border_1()
            .border_color(ShellDeckColors::border().opacity(0.45))
            .bg(ShellDeckColors::bg_sidebar())
            .shadow(vec![use_theme().tokens.shadow_sm].into())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(12.0))
                    .py(px(4.0))
                    .rounded_full()
                    .bg(ShellDeckColors::success().opacity(0.12))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::success())
                    .child(lucide_icon("check", 13.0, ShellDeckColors::success()))
                    .child(t!("account.splash.signed_in").to_string()),
            )
            .child(div().mt(px(18.0)).child(mascot))
            .child(
                div()
                    .mt(px(18.0))
                    .text_size(px(28.0))
                    .line_height(px(34.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .text_center()
                    .child(
                        t!(
                            "account.splash.welcome",
                            name = splash.display_name.as_str()
                        )
                        .to_string(),
                    ),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .text_size(px(15.0))
                    .text_color(ShellDeckColors::text_muted())
                    .text_center()
                    .child(t!("account.splash.preparing").to_string()),
            )
            .child(
                div()
                    .mt(px(28.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w(px(BAR_W))
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("account.splash.syncing").to_string())
                            .child(progress_percentage),
                    )
                    .child(progress_bar),
            );

        div()
            .id("post-login-splash")
            .occlude()
            .absolute()
            // Under the title bar, which shows its signed-out face meanwhile.
            .top(px(WORKSPACE_TITLEBAR_HEIGHT))
            .left_0()
            .right_0()
            .bottom_0()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .child(
                super::welcome::welcome_backdrop(is_maximized)
                    .child(super::welcome::welcome_foot(is_maximized))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom(px(super::welcome::FOOTER_H))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(card),
                    ),
            )
            .with_animation(
                SharedString::from(format!(
                    "post-login-splash-{}",
                    if splash.dismissing {
                        "fade-out"
                    } else {
                        "visible"
                    }
                )),
                Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_FADE_MS))
                    .with_easing(ease_in_out),
                {
                    let dismissing = splash.dismissing;
                    move |el, delta| el.opacity(post_login_splash_opacity(dismissing, delta))
                },
            )
    }
}
