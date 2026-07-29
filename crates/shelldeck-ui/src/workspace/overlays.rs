use super::*;
use crate::monolith::{MonolithMotion, animated_monolith};

impl Workspace {
    /// User-mode "Demander à JeanClaude" card: a composer that files a request
    /// through Jean's Slack intake, plus a read-only recent-activity list.
    pub(super) fn render_jean_ask_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let input_display = if self.jean_ask_input.is_empty() {
            div()
                .text_color(ShellDeckColors::text_muted())
                .child(t!("user.jean.ask_placeholder").to_string())
        } else {
            div()
                .text_color(ShellDeckColors::text_primary())
                .child(self.jean_ask_input.clone())
        };

        let mut activity = div().flex().flex_col().gap(px(2.0)).mt(px(6.0));
        if let Some(state) = &self.jean_state {
            for t in state.tickets.iter().take(10) {
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
                                .child(t.status.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t.prompt.clone()),
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
                            .child(t!("user.jean.ask_title").to_string()),
                    ),
            )
            .child(
                div()
                    .id("jean-ask-input")
                    .track_focus(&self.jean_ask_focus)
                    .on_key_down(
                        cx.listener(|this, e: &KeyDownEvent, _w, cx| {
                            this.handle_jean_ask_key(e, cx)
                        }),
                    )
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
                            .child(t!("user.jean.confirm_hint").to_string()),
                    )
                    .child(
                        div()
                            .id("jean-ask-send")
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
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.submit_jean_ask(cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .mt(px(4.0))
                    .child(t!("user.jean.recent_activity").to_string()),
            )
            .child(activity)
    }

    pub(super) fn render_post_login_splash(&self, splash: &PostLoginSplash) -> impl IntoElement {
        use std::time::Duration;

        let mascot = animated_monolith("post-login-mascot", 188.0, MonolithMotion::FloatAndBreathe);

        let progress_bar = div()
            .relative()
            .w(px(220.0))
            .h(px(5.0))
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
                        |el, delta| el.w(px(220.0 * post_login_simulated_progress(delta))),
                    ),
            );

        let progress_percentage = div()
            .min_w(px(34.0))
            .text_right()
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::primary())
            .with_animation(
                "post-login-progress-percentage",
                Animation::new(Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS)),
                |el, delta| {
                    let percentage = (post_login_simulated_progress(delta) * 100.0).round() as u8;
                    el.child(format!("{percentage}%"))
                },
            );

        div()
            .id("post-login-splash")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(ShellDeckColors::bg_primary())
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .absolute()
                    .w(px(420.0))
                    .h(px(420.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary().opacity(0.07)),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .max_w(px(480.0))
                    .px(px(28.0))
                    .child(mascot)
                    .child(
                        div()
                            .mt(px(22.0))
                            .text_size(px(25.0))
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
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .text_center()
                            .child(t!("account.splash.preparing").to_string()),
                    )
                    .child(
                        div()
                            .mt(px(22.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(220.0))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("account.splash.syncing").to_string())
                                    .child(progress_percentage),
                            )
                            .child(progress_bar),
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
