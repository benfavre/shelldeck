//! Shared conversation primitives for Support tickets and hosted requests.
//!
//! Both surfaces display the same semantic objects. Keeping their chrome here
//! prevents the old ticket bubbles and the newer request prose from drifting
//! apart again.

use crate::i18n::rel_time;
use crate::icons::lucide_icon;
use crate::markdown::{MarkdownLinkAction, MarkdownLinkHandler};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::prelude::Markdown;
use gpui::prelude::*;
use gpui::*;
use pulldown_cmark::{Event, Options, Parser};
use shelldeck_core::config::issues::IssueAttachment;
use std::ops::Range;
use std::rc::Rc;

#[derive(Clone, Copy)]
pub(crate) enum ThreadNoteKind {
    Status,
    System,
    Github,
    Dispatch,
    Internal,
}

#[derive(Default)]
pub(crate) struct ThreadMessageExtras {
    pub quote: Option<AnyElement>,
    pub delivery: Option<AnyElement>,
    pub actions: Option<AnyElement>,
    pub group: Option<SharedString>,
    pub link_handler: Option<MarkdownLinkHandler>,
}

pub(crate) type ThreadLinkHandler = MarkdownLinkHandler;
pub(crate) type ThreadLinkAction = MarkdownLinkAction;

/// Cursor-anchored confirmation shared by Ticket, Support Request and User
/// Request threads. Markdown owns link detection/styling; this component owns
/// the deliberate open/copy decision and the external-domain warning.
pub(crate) fn thread_link_popover(
    action: ThreadLinkAction,
    on_close: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let on_close = Rc::new(on_close);
    let external = !action.internal;
    let host = action.host.clone();
    let url_for_copy = action.url.clone();
    let url_for_open = action.url.clone();
    let close_backdrop = on_close.clone();
    let close_copy = on_close.clone();
    let close_open = on_close.clone();

    let card = div()
        .occlude()
        .w(px(340.0))
        .max_w(relative(0.92))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(ShellDeckColors::border())
        .bg(ShellDeckColors::bg_surface())
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(lucide_icon(
                    if external {
                        "triangle-alert"
                    } else {
                        "external-link"
                    },
                    15.0,
                    if external {
                        ShellDeckColors::warning()
                    } else {
                        ShellDeckColors::primary()
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(if external {
                            t!("support.thread.link.external_title").to_string()
                        } else {
                            t!("support.thread.link.internal_title").to_string()
                        }),
                ),
        )
        .when(external, |card| {
            card.child(
                div()
                    .px(px(9.0))
                    .py(px(8.0))
                    .rounded(px(7.0))
                    .bg(ShellDeckColors::warning().opacity(0.10))
                    .text_size(px(11.0))
                    .line_height(relative(1.4))
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("support.thread.link.external_warning").to_string()),
            )
        })
        .child(
            div()
                .min_w(px(0.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ShellDeckColors::text_muted())
                .child(host),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .child(
                    Button::new(
                        "thread-link-copy",
                        t!("support.thread.link.copy").to_string(),
                    )
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("copy"))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url_for_copy.clone()));
                        close_copy(cx);
                    }),
                )
                .child(
                    Button::new(
                        "thread-link-open",
                        t!("support.thread.link.open").to_string(),
                    )
                    .variant(ButtonVariant::Default)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("external-link"))
                    .on_click(move |_, _, cx| {
                        cx.open_url(&url_for_open);
                        close_open(cx);
                    }),
                ),
        );

    div()
        .absolute()
        .inset_0()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| close_backdrop(cx))
        .child(
            deferred(
                anchored()
                    .snap_to_window()
                    // Keep the action panel above the selected link. Its
                    // bottom-left corner sits just above the pointer; window
                    // snapping remains the fallback for links near the top.
                    .anchor(Corner::BottomLeft)
                    .position(action.position)
                    .offset(point(gpui::px(0.0), gpui::px(-8.0)))
                    .child(card),
            )
            .with_priority(10),
        )
        .into_any_element()
}

#[derive(Clone, Copy)]
pub(super) enum ThreadDeliveryTone {
    Success,
    Error,
}

pub(super) fn thread_status_color(status: &str) -> Hsla {
    match status {
        "in_progress" | "triaging" | "pending" => ShellDeckColors::warning(),
        "blocked" => ShellDeckColors::error(),
        "done" | "closed" => ShellDeckColors::success(),
        _ => ShellDeckColors::primary(),
    }
}

pub(super) fn thread_priority_color(priority: &str) -> Hsla {
    match priority {
        "urgent" => ShellDeckColors::error(),
        "high" => ShellDeckColors::warning(),
        "low" => ShellDeckColors::text_muted(),
        _ => ShellDeckColors::primary(),
    }
}

/// Compact metadata trigger shared by Request and Ticket headers. Source
/// adapters choose the labels and actions; this owns the 24 px geometry.
pub(super) fn thread_header_picker(
    id: &'static str,
    marker: impl IntoElement,
    label: impl Into<SharedString>,
    interactive: bool,
) -> AnyElement {
    div()
        .id(id)
        .h(px(24.0))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(4.0))
        .px(px(7.0))
        .rounded(px(6.0))
        .bg(ShellDeckColors::bg_surface())
        .text_size(px(10.5))
        .text_color(ShellDeckColors::text_muted())
        .when(interactive, |picker| {
            picker
                .cursor_pointer()
                .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        })
        .child(marker)
        .child(label.into())
        .when(interactive, |picker| {
            picker.child(lucide_icon(
                "chevron-down",
                10.0,
                ShellDeckColors::text_muted(),
            ))
        })
        .into_any_element()
}

/// Compact option row for the metadata popovers shared by both Support data
/// sources. The optional subtitle is used by the large assignee directory.
pub(super) fn thread_picker_option_row(
    id: SharedString,
    marker: impl IntoElement,
    label: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    active: bool,
) -> Stateful<Div> {
    let mut row = div()
        .id(ElementId::from(id))
        .w_full()
        .min_h(px(if subtitle.is_some() { 40.0 } else { 30.0 }))
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .text_color(ShellDeckColors::text_primary())
        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        .child(marker)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(11.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label.into()),
                )
                .when_some(subtitle, |column, subtitle| {
                    column.child(
                        div()
                            .text_size(px(9.5))
                            .text_color(ShellDeckColors::text_muted())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(subtitle),
                    )
                }),
        );
    if active {
        row = row.bg(ShellDeckColors::selected_bg()).child(lucide_icon(
            "check",
            12.0,
            ShellDeckColors::primary(),
        ));
    }
    row
}

pub(super) fn timeline_day(at: f64) -> Option<chrono::NaiveDate> {
    chrono::DateTime::from_timestamp_millis(at as i64).map(|value| value.date_naive())
}

pub(super) fn timeline_day_label(at: f64) -> String {
    let Some(day) = timeline_day(at) else {
        return String::new();
    };
    let today = chrono::Utc::now().date_naive();
    if day == today {
        t!("support.thread.today").to_string()
    } else if day == today.pred_opt().unwrap_or(today) {
        t!("support.thread.yesterday").to_string()
    } else {
        day.format("%d/%m/%Y").to_string()
    }
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

fn attachment_basename(filename: &str) -> &str {
    filename
        .rsplit(|character| character == '/' || character == '\\')
        .next()
        .unwrap_or(filename)
        .trim()
}

/// Replace Outlook/Office inline-image transport markers with reader-facing
/// text. The source payload remains untouched; this adapter only feeds the
/// conversation renderer. When Manage exposes the matching attachment, its
/// canonical filename points the reader to the gallery rendered below the
/// message. Otherwise no opaque Content-ID escapes into the UI.
pub(crate) fn cid_safe_text(source: &str, attachments: &[IssueAttachment]) -> String {
    let folded = source.to_ascii_lowercase();
    let mut rendered = String::with_capacity(source.len());
    let mut cursor = 0usize;

    while let Some(relative_start) = folded[cursor..].find("[cid:") {
        let start = cursor + relative_start;
        rendered.push_str(&source[cursor..start]);

        let token_start = start + "[cid:".len();
        let Some(relative_end) = source[token_start..].find(']') else {
            rendered.push_str(&source[start..]);
            cursor = source.len();
            break;
        };
        let end = token_start + relative_end;
        let marker_filename = source[token_start..end]
            .split('@')
            .next()
            .map(str::trim)
            .unwrap_or_default();
        let matching_filename = (!marker_filename.is_empty())
            .then(|| {
                attachments.iter().find_map(|attachment| {
                    let filename = attachment_basename(&attachment.filename);
                    (!filename.is_empty() && filename.eq_ignore_ascii_case(marker_filename))
                        .then_some(filename)
                })
            })
            .flatten();

        if let Some(filename) = matching_filename {
            rendered.push_str(&t!("support.thread.cid.attached", filename = filename).to_string());
        } else {
            rendered.push_str(&t!("support.thread.cid.unavailable").to_string());
        }
        cursor = end + 1;
    }

    if cursor < source.len() {
        rendered.push_str(&source[cursor..]);
    }
    rendered
}

/// Split Markdown at top-level block boundaries. The thread's native GPUI
/// list can then virtualise a huge message block-by-block instead of parsing,
/// laying out and painting the complete document on every scroll tick.
pub(super) fn markdown_blocks(source: &str, attachments: &[IssueAttachment]) -> Vec<SharedString> {
    let source = cid_safe_text(source, attachments);
    let source = source.as_str();
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

fn markdown_body(
    body: SharedString,
    base_font_size: Pixels,
    link_handler: Option<ThreadLinkHandler>,
) -> Div {
    let mut markdown = Markdown::new(body)
        .base_font_size(base_font_size)
        .compact()
        .w_full()
        .min_w(px(0.0))
        .whitespace_normal();
    if let Some(handler) = link_handler {
        markdown = markdown.on_link_click(move |url, window, cx| handler(url, window, cx));
    }
    div()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .whitespace_normal()
        .child(markdown)
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
    let body = body.into();
    let body = cid_safe_text(body.as_ref(), &[]);
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
        .child(div().whitespace_normal().italic().child(body))
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

/// Shared visual shell for a generated reply awaiting review. Requests and
/// Tickets keep separate actions/data adapters, but the proposal itself must
/// not acquire two subtly different layouts.
pub(super) fn ai_draft_card(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    leading_actions: Vec<AnyElement>,
    trailing_actions: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .w_full()
        .max_w(px(560.0))
        .min_w(px(0.0))
        .overflow_hidden()
        .gap(px(8.0))
        .p(px(11.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(ShellDeckColors::primary().opacity(0.40))
        .bg(ShellDeckColors::primary().opacity(0.08))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.0))
                .text_size(px(11.0))
                .text_color(ShellDeckColors::primary())
                .child(lucide_icon("sparkles", 12.0, ShellDeckColors::primary()))
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(12.5))
                .line_height(relative(1.55))
                .text_color(ShellDeckColors::text_primary())
                .whitespace_normal()
                .child(body.into()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .children(leading_actions)
                .child(div().flex_1())
                .children(trailing_actions),
        )
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
pub(crate) struct HumanMessageMeta {
    pub author: SharedString,
    pub mine: bool,
    pub at: f64,
    pub channel: Option<SharedString>,
}

pub(crate) fn human_message(
    meta: HumanMessageMeta,
    body: impl Into<SharedString>,
    attachments: Option<AnyElement>,
    mut extras: ThreadMessageExtras,
    base_font_size: Pixels,
) -> AnyElement {
    let HumanMessageMeta {
        author,
        mine,
        at,
        channel,
    } = meta;
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
                .child(author),
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
                .child(channel),
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
    // Attachment-only messages are valid. Do not manufacture a blank prose
    // row between their identity and gallery.
    if !body.trim().is_empty() {
        message = message.child(markdown_body(
            body,
            base_font_size,
            extras.link_handler.clone(),
        ));
    }

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
        .child(markdown_body(
            body.into(),
            base_font_size,
            extras.link_handler.clone(),
        ));
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
pub(crate) fn note(
    body: impl Into<SharedString>,
    actor: Option<impl Into<SharedString>>,
    at: f64,
    kind: ThreadNoteKind,
    base_font_size: Pixels,
) -> AnyElement {
    let body: SharedString = cid_safe_text(body.into().as_ref(), &[]).into();
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
                    markdown_body(body.clone(), base_font_size, None)
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
    use super::{cid_safe_text, markdown_blocks};
    use shelldeck_core::config::issues::IssueAttachment;

    #[test]
    fn markdown_is_split_on_complete_top_level_blocks() {
        let source =
            "# Titre\n\nPremier **paragraphe**.\n\n- un\n- deux\n\n```rust\nfn main() {}\n```";
        let blocks = markdown_blocks(source, &[]);
        assert!(blocks.len() >= 4, "expected virtualisable Markdown blocks");
        assert!(blocks.iter().any(|block| block.contains("**paragraphe**")));
        assert!(blocks.iter().any(|block| block.contains("- un")));
        assert!(blocks.iter().any(|block| block.contains("```rust")));
        assert!(blocks.iter().all(|block| !block.trim().is_empty()));
    }

    #[test]
    fn plain_text_remains_renderable() {
        let blocks = markdown_blocks("Une seule ligne sans balisage", &[]);
        assert_eq!(blocks.as_slice(), &["Une seule ligne sans balisage"]);
        assert!(markdown_blocks("  \n", &[]).is_empty());
    }

    // SDTEST-1730 — Content-ID transport markers never escape into visible
    // conversation prose. A same-message attachment resolves by basename;
    // an unresolved image remains explicit without exposing its opaque id.
    #[test]
    fn cid_markers_resolve_to_attachment_names_or_a_readable_fallback() {
        let attachments = vec![IssueAttachment {
            filename: "mail/image003.png".to_string(),
            ..Default::default()
        }];
        let rendered = cid_safe_text(
            "Avant\n[CID:image003.png@01DD36D8.282608C0]\n[cid:image001.png@opaque]\nAprès",
            &attachments,
        );

        assert!(rendered.contains("image003.png"));
        assert!(!rendered.to_ascii_lowercase().contains("cid:"));
        assert!(!rendered.contains("01DD36D8"));
        assert!(!rendered.contains("opaque"));
        assert!(rendered.starts_with("Avant\n"));
        assert!(rendered.ends_with("\nAprès"));
        assert_eq!(
            cid_safe_text("Texte [cid:image.png sans fin", &[]),
            "Texte [cid:image.png sans fin"
        );
    }
}
