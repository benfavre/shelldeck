//! Shared conversation primitives for Support tickets and hosted requests.
//!
//! Both surfaces display the same semantic objects. Keeping their chrome here
//! prevents the old ticket bubbles and the newer request prose from drifting
//! apart again.

use crate::i18n::rel_time;
use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::prelude::Markdown;
use gpui::prelude::*;
use gpui::*;
use pulldown_cmark::{Event, Options, Parser};
use std::ops::Range;

#[derive(Clone, Copy)]
pub(super) enum ThreadNoteKind {
    Status,
    System,
    Github,
    Dispatch,
    Internal,
}

#[derive(Default)]
pub(super) struct ThreadMessageExtras {
    pub quote: Option<AnyElement>,
    pub delivery: Option<AnyElement>,
    pub actions: Option<AnyElement>,
    pub group: Option<SharedString>,
}

#[derive(Clone, Copy)]
pub(super) enum ThreadDeliveryTone {
    Success,
    Error,
}

fn styled_body(
    body: &str,
    font_size: f32,
    line_height: f32,
    color: Hsla,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Div {
    let text = SharedString::from(if body.is_empty() { " " } else { body }.to_string());
    let body = div()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .whitespace_normal()
        .line_height(relative(line_height))
        .text_size(px(font_size))
        .text_color(color);

    if highlights.is_empty() {
        body.child(text)
    } else {
        body.child(StyledText::new(text).with_highlights(highlights.to_vec()))
    }
}

/// Split Markdown at top-level block boundaries. The thread's native GPUI
/// list can then virtualise a huge message block-by-block instead of parsing,
/// laying out and painting the complete document on every scroll tick.
pub(super) fn markdown_blocks(source: &str) -> Vec<SharedString> {
    if source.trim().is_empty() {
        return Vec::new();
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut depth = 0usize;
    let mut chunk_start = 0usize;
    let mut chunks = Vec::new();
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        let boundary = match event {
            Event::Start(_) => {
                depth += 1;
                None
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                (depth == 0).then_some(range.end)
            }
            Event::Rule if depth == 0 => Some(range.end),
            _ => None,
        };

        if let Some(end) = boundary.filter(|end| *end > chunk_start) {
            let chunk = source[chunk_start..end].trim();
            if !chunk.is_empty() {
                chunks.push(SharedString::from(chunk.to_string()));
            }
            chunk_start = end;
        }
    }

    let tail = source[chunk_start..].trim();
    if !tail.is_empty() {
        chunks.push(SharedString::from(tail.to_string()));
    }
    if chunks.is_empty() {
        chunks.push(SharedString::from(source.to_string()));
    }
    chunks
}

fn markdown_body(body: SharedString, base_font_size: Pixels) -> Div {
    div()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .whitespace_normal()
        .child(
            Markdown::new(body)
                .base_font_size(base_font_size)
                .compact()
                .w_full()
                .min_w(px(0.0))
                .whitespace_normal(),
        )
}

fn looks_like_markdown(body: &str) -> bool {
    body.contains("**")
        || body.contains('`')
        || body.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with('#')
                || line.starts_with("- ")
                || line.starts_with("* ")
                || line.starts_with("> ")
                || line.starts_with("1. ")
        })
}

pub(super) fn attributed_quote(
    author: impl Into<SharedString>,
    body: impl Into<SharedString>,
) -> AnyElement {
    div()
        .w_full()
        .min_w(px(0.0))
        .pl(px(10.0))
        .py(px(4.0))
        .border_l(px(2.0))
        .border_color(ShellDeckColors::border())
        .text_size(px(11.5))
        .line_height(relative(1.45))
        .text_color(ShellDeckColors::text_muted())
        .child(
            div()
                .whitespace_normal()
                .child(t!("support.thread.reply_to", author = author.into()).to_string()),
        )
        .child(div().whitespace_normal().italic().child(body.into()))
        .into_any_element()
}

pub(super) fn delivery_status(
    label: impl Into<SharedString>,
    tone: ThreadDeliveryTone,
    retry: Option<AnyElement>,
) -> AnyElement {
    let color = match tone {
        ThreadDeliveryTone::Success => ShellDeckColors::text_muted(),
        ThreadDeliveryTone::Error => ShellDeckColors::error(),
    };
    let icon_color = match tone {
        ThreadDeliveryTone::Success => ShellDeckColors::success(),
        ThreadDeliveryTone::Error => ShellDeckColors::error(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .mt(px(4.0))
        .text_size(px(10.0))
        .text_color(color)
        .child(lucide_icon(
            if matches!(tone, ThreadDeliveryTone::Success) {
                "check"
            } else {
                "circle-alert"
            },
            12.0,
            icon_color,
        ))
        .child(label.into())
        .children(retry)
        .into_any_element()
}

pub(super) fn day_separator(label: impl Into<SharedString>) -> AnyElement {
    let line = || {
        div()
            .flex_1()
            .min_w(px(12.0))
            .h(px(1.0))
            .bg(ShellDeckColors::border())
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .my(px(4.0))
        .text_size(px(10.0))
        .tracking_widest()
        .text_color(ShellDeckColors::text_muted())
        .child(line())
        .child(label.into())
        .child(line())
        .into_any_element()
}

pub(super) fn typing_indicator(author: impl Into<SharedString>) -> AnyElement {
    let dots = div()
        .flex()
        .items_center()
        .gap(px(3.0))
        .children((0..3).map(|_| {
            div()
                .size(px(5.0))
                .rounded_full()
                .bg(ShellDeckColors::text_muted().opacity(0.55))
        }));
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(11.0))
        .text_color(ShellDeckColors::text_muted())
        .child(dots)
        .child(t!("support.thread.typing", author = author.into()).to_string())
        .into_any_element()
}

pub(super) fn local_draft(body: impl Into<SharedString>) -> AnyElement {
    div()
        .ml_auto()
        .max_w(relative(1.0))
        .min_w(px(0.0))
        .px(px(11.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_dashed()
        .border_color(ShellDeckColors::border())
        .text_size(px(12.0))
        .line_height(relative(1.45))
        .text_color(ShellDeckColors::text_muted())
        .child(t!("support.thread.local_draft", body = body.into()).to_string())
        .into_any_element()
}

/// Compact contextual action used only inside a message's hover toolbar.
/// adabraka's smallest labeled Button is 36 px high, which is taller than the
/// 18 px identity row this toolbar overlays, so this one-off density needs a
/// shared thread primitive instead of a regular Button.
pub(super) fn message_action(
    id: impl Into<ElementId>,
    icon: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.0))
        .h(px(22.0))
        .px(px(6.0))
        .rounded(px(5.0))
        .bg(ShellDeckColors::bg_surface())
        .border_1()
        .border_color(ShellDeckColors::border())
        .cursor_pointer()
        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        .child(lucide_icon(icon, 11.0, ShellDeckColors::text_muted()))
        .child(
            div()
                .text_size(px(10.5))
                .text_color(ShellDeckColors::text_muted())
                .child(label.into()),
        )
        .on_click(on_click)
        .into_any_element()
}

/// A human-authored message in the thread. The author's colour identifies our
/// own voice; alignment and framing deliberately do not, so both Support data
/// sources read as one continuous conversation.
pub(super) fn human_message(
    author: impl Into<SharedString>,
    mine: bool,
    at: f64,
    channel: Option<impl Into<SharedString>>,
    body: impl Into<SharedString>,
    attachments: Option<AnyElement>,
    mut extras: ThreadMessageExtras,
    base_font_size: Pixels,
) -> AnyElement {
    let body: SharedString = body.into();
    let identity = div()
        .flex()
        .items_baseline()
        .gap(px(7.0))
        .child(
            div()
                // Keep the prototype's 12 px label size; weight, not a larger
                // font box, distinguishes the author from the prose below.
                .text_size(px(12.0))
                .line_height(relative(1.0))
                .font_weight(FontWeight::EXTRA_BOLD)
                .tracking_tight()
                .text_color(if mine {
                    ShellDeckColors::primary()
                } else {
                    ShellDeckColors::text_primary()
                })
                .child(author.into()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(10.5))
                .line_height(relative(1.0))
                .text_color(ShellDeckColors::text_muted())
                .child(rel_time(at)),
        );

    let mut head = div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap(px(7.0))
        .child(identity);

    if let Some(channel) = channel {
        head = head.child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .h(px(18.0))
                .px(px(6.0))
                .rounded_full()
                .bg(ShellDeckColors::bg_primary())
                .text_size(px(10.0))
                .line_height(relative(1.0))
                .text_color(ShellDeckColors::text_muted())
                .child(channel.into()),
        );
    }

    let group = extras
        .group
        .take()
        .unwrap_or_else(|| SharedString::from("thread-message"));
    let mut message = div()
        .relative()
        .group(group.clone())
        .flex()
        .flex_col()
        .gap(px(6.0))
        .w_full()
        .max_w(px(560.0))
        .min_w(px(0.0))
        .child(head);

    if let Some(quote) = extras.quote.take() {
        message = message.child(quote);
    }
    message = message.child(markdown_body(body, base_font_size));

    if let Some(attachments) = attachments {
        message = message.child(attachments);
    }
    if let Some(delivery) = extras.delivery.take() {
        message = message.child(delivery);
    }
    if let Some(actions) = extras.actions.take() {
        message = message.child(
            div()
                .absolute()
                .top(px(-4.0))
                .right(px(0.0))
                .opacity(0.0)
                .group_hover(group, |style| style.opacity(1.0))
                .child(actions),
        );
    }

    message.into_any_element()
}

/// A later virtualised Markdown block belonging to the same human message.
/// It omits the repeated author row while keeping the same prose width.
pub(super) fn human_message_continuation(
    body: impl Into<SharedString>,
    attachments: Option<AnyElement>,
    mut extras: ThreadMessageExtras,
    base_font_size: Pixels,
) -> AnyElement {
    let mut message = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .w_full()
        .max_w(px(560.0))
        .min_w(px(0.0))
        .child(markdown_body(body.into(), base_font_size));
    if let Some(attachments) = attachments {
        message = message.child(attachments);
    }
    if let Some(delivery) = extras.delivery.take() {
        message = message.child(delivery);
    }
    message.into_any_element()
}

/// A machine/internal event. Unlike a human answer it keeps a compact tinted
/// frame because it describes state rather than participating in the prose.
pub(super) fn note(
    body: impl Into<SharedString>,
    actor: Option<impl Into<SharedString>>,
    at: f64,
    kind: ThreadNoteKind,
    base_font_size: Pixels,
) -> AnyElement {
    let body: SharedString = body.into();
    let actor = actor.map(Into::into);
    let (icon, border, bg) = match kind {
        ThreadNoteKind::Status => (
            "check",
            ShellDeckColors::primary().opacity(0.30),
            ShellDeckColors::primary().opacity(0.08),
        ),
        ThreadNoteKind::Github => (
            "git-branch",
            ShellDeckColors::border(),
            ShellDeckColors::bg_primary(),
        ),
        ThreadNoteKind::Dispatch => (
            "server",
            ShellDeckColors::success().opacity(0.30),
            ShellDeckColors::success().opacity(0.10),
        ),
        ThreadNoteKind::System | ThreadNoteKind::Internal => (
            if matches!(kind, ThreadNoteKind::Internal) {
                "sticky-note"
            } else {
                "info"
            },
            ShellDeckColors::warning().opacity(0.30),
            ShellDeckColors::warning().opacity(0.10),
        ),
    };

    let actor_is_in_body = actor
        .as_ref()
        .is_some_and(|actor: &SharedString| body.starts_with(actor.as_ref()));
    let metadata = match actor.as_ref().filter(|_| !actor_is_in_body) {
        Some(actor) => format!("{} · {}", rel_time(at), actor),
        None => rel_time(at),
    };

    // The prototype uses weight to expose the information-bearing fragments:
    // who acted, which state changed, which external object/runtime is linked.
    // StyledText keeps those runs inline without splitting the wrapping line.
    let mut bold_ranges = Vec::new();
    if actor_is_in_body {
        if let Some(actor) = &actor {
            bold_ranges.push(0..actor.len());
        }
    }
    match kind {
        ThreadNoteKind::Status => {
            for label in [
                "À traiter",
                "En cours",
                "Résolue",
                "Résolu",
                "Fermée",
                "Ouverte",
                "Open",
                "Pending",
                "Closed",
            ] {
                if let Some(start) = body.find(label) {
                    bold_ranges.push(start..start + label.len());
                }
            }
        }
        ThreadNoteKind::Github => {
            if let Some(start) = body.find("Liée à ").map(|start| start + "Liée à ".len()) {
                let end = body[start..]
                    .find(" —")
                    .map(|end| start + end)
                    .unwrap_or(body.len());
                bold_ranges.push(start..end);
            }
        }
        ThreadNoteKind::Dispatch => {
            if let Some(start) = body
                .find("Dispatché vers ")
                .map(|start| start + "Dispatché vers ".len())
            {
                let end = body[start..]
                    .find(" —")
                    .map(|end| start + end)
                    .unwrap_or(body.len());
                bold_ranges.push(start..end);
            }
            if let Some(start) = body.find("script ").map(|start| start + "script ".len()) {
                bold_ranges.push(start..body.len());
            }
        }
        ThreadNoteKind::System | ThreadNoteKind::Internal => {}
    }
    bold_ranges.sort_by_key(|range| range.start);
    let mut last_end = 0;
    let bold = HighlightStyle {
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    let highlights = bold_ranges
        .into_iter()
        .filter_map(|range| {
            if range.start < last_end || range.is_empty() {
                None
            } else {
                last_end = range.end;
                Some((range, bold))
            }
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .items_start()
        .gap(px(10.0))
        .w_full()
        .max_w(px(560.0))
        .min_w(px(0.0))
        .p(px(9.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .child(
            div()
                .flex_shrink_0()
                .size(px(22.0))
                .rounded(px(5.0))
                .bg(ShellDeckColors::bg_surface())
                .flex()
                .items_center()
                .justify_center()
                .child(lucide_icon(icon, 13.0, ShellDeckColors::text_muted())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(if looks_like_markdown(body.as_ref()) {
                    markdown_body(body.clone(), base_font_size)
                } else {
                    styled_body(
                        body.as_ref(),
                        11.5,
                        1.4,
                        ShellDeckColors::text_primary(),
                        &highlights,
                    )
                })
                .child(
                    div()
                        .mt(px(2.0))
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(metadata),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::markdown_blocks;

    #[test]
    fn markdown_is_split_on_complete_top_level_blocks() {
        let source =
            "# Titre\n\nPremier **paragraphe**.\n\n- un\n- deux\n\n```rust\nfn main() {}\n```";
        let blocks = markdown_blocks(source);
        assert!(blocks.len() >= 4, "expected virtualisable Markdown blocks");
        assert!(blocks.iter().any(|block| block.contains("**paragraphe**")));
        assert!(blocks.iter().any(|block| block.contains("- un")));
        assert!(blocks.iter().any(|block| block.contains("```rust")));
        assert!(blocks.iter().all(|block| !block.trim().is_empty()));
    }

    #[test]
    fn plain_text_remains_renderable() {
        let blocks = markdown_blocks("Une seule ligne sans balisage");
        assert_eq!(blocks.as_slice(), &["Une seule ligne sans balisage"]);
        assert!(markdown_blocks("  \n").is_empty());
    }
}
