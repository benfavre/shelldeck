use crate::{scale::px, t, theme::ShellDeckColors};
use adabraka_ui::prelude::{Button, ButtonSize, ButtonVariant};
use gpui::{div, prelude::*, ClipboardItem, Div};

/// The provider CLI owns login and credentials, on the host where it runs.
/// Showing setup never starts a login or a model request.
pub(crate) fn render_subscription_setup(command: &str, remote: bool) -> Option<Div> {
    let (description, login, status, docs) = match command {
        "claude" => (
            t!("ai.subscription.claude").to_string(),
            "claude auth login --claudeai",
            "claude auth status",
            "https://code.claude.com/docs/en/authentication",
        ),
        "codex" => (
            t!("ai.subscription.codex").to_string(),
            "codex login",
            "codex login status",
            "https://developers.openai.com/codex/auth",
        ),
        _ => return None,
    };
    let location = if remote {
        t!("ai.subscription.remote")
    } else {
        t!("ai.subscription.local")
    };
    Some(
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.0))
            .gap(px(8.0))
            .p(px(12.0))
            .rounded_md()
            .border_1()
            .border_color(ShellDeckColors::border())
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary())
            .child(description)
            .child(location.to_string())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().font_family("JetBrains Mono").child(login))
                    .child(
                        Button::new(
                            "subscription-copy-login",
                            t!("ai.subscription.copy").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(login.to_string()));
                        }),
                    )
                    .child(
                        Button::new(
                            "subscription-login-docs",
                            t!("ai.subscription.docs").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| cx.open_url(docs)),
                    ),
            )
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("ai.subscription.status", command = status).to_string()),
            )
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("ai.subscription.billing").to_string()),
            ),
    )
}
