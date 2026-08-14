//! Security boundary and shared interaction chrome for untrusted Markdown.
//!
//! The vendored renderer is deliberately passive: it never fetches Markdown
//! images and never opens a link without a host-provided callback. This module
//! is the second half of that boundary. It accepts only absolute HTTP(S) URLs,
//! then presents the same explicit copy/open choice on every rich-text surface.

use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use gpui::prelude::*;
use gpui::*;
use std::rc::Rc;
use url::Url;

const MAX_MARKDOWN_URL_LEN: usize = 8 * 1024;
const INTERNAL_DOMAIN_ROOTS: [&str; 3] = ["inklura.fr", "bext.dev", "shelldeck.1clic.pro"];

pub(crate) type MarkdownLinkHandler = Rc<dyn Fn(&str, &mut Window, &mut App) + 'static>;

#[derive(Clone)]
pub(crate) struct MarkdownLinkAction {
    pub(crate) url: String,
    pub(crate) host: String,
    pub(crate) internal: bool,
    pub(crate) position: Point<Pixels>,
}

impl MarkdownLinkAction {
    /// Validate an untrusted Markdown destination before it reaches either the
    /// clipboard or the operating system's URL opener.
    pub(crate) fn new(url: &str, position: Point<Pixels>) -> Option<Self> {
        let (url, host, internal) = validated_markdown_link(url)?;
        Some(Self {
            url,
            host,
            internal,
            position,
        })
    }
}

fn validated_markdown_link(raw: &str) -> Option<(String, String, bool)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_MARKDOWN_URL_LEN || raw.chars().any(char::is_control) {
        return None;
    }

    let parsed = Url::parse(raw).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let host = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let internal = INTERNAL_DOMAIN_ROOTS
        .iter()
        .any(|root| host == *root || host.ends_with(&format!(".{root}")));
    Some((raw.to_string(), host, internal))
}

#[cfg(test)]
pub(crate) fn markdown_link_is_safe(url: &str) -> bool {
    validated_markdown_link(url).is_some()
}

#[cfg(test)]
pub(crate) fn markdown_link_is_internal(url: &str) -> bool {
    validated_markdown_link(url).is_some_and(|(_, _, internal)| internal)
}

/// Cursor-anchored confirmation shared by every dynamic Markdown surface.
/// Link detection/styling stays in adabraka-ui; this component owns the
/// deliberate open/copy decision and the external-domain warning.
pub(crate) fn markdown_link_popover(
    action: MarkdownLinkAction,
    on_close: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let on_close = Rc::new(on_close);
    let external = !action.internal;
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
                .child(action.host),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .child(
                    Button::new(
                        "markdown-link-copy",
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
                        "markdown-link-open",
                        t!("support.thread.link.open").to_string(),
                    )
                    .variant(ButtonVariant::Default)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("external-link"))
                    .on_click(move |_, _, cx| {
                        // `MarkdownLinkAction::new` is the mandatory gate for
                        // this value, so only an absolute HTTP(S) URL can land
                        // in the platform opener.
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
                    .anchor(Corner::BottomLeft)
                    .position(action.position)
                    .offset(point(gpui::px(0.0), gpui::px(-8.0)))
                    .child(card),
            )
            .with_priority(10),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{markdown_link_is_internal, markdown_link_is_safe};

    // SDTEST-1604
    #[test]
    fn markdown_links_only_accept_absolute_http_urls_without_credentials() {
        for safe in [
            "https://example.com/path?q=1#part",
            "http://127.0.0.1:8080/status",
        ] {
            assert!(markdown_link_is_safe(safe), "{safe}");
        }
        for unsafe_url in [
            "javascript:alert(1)",
            "data:text/html,boom",
            "file:///etc/passwd",
            "shelldeck://terminal/new",
            "/relative/path",
            "https://user:pass@example.com/private",
            "https://",
        ] {
            assert!(!markdown_link_is_safe(unsafe_url), "{unsafe_url}");
        }
    }

    // SDTEST-1605
    #[test]
    fn ecosystem_domains_require_an_exact_host_or_subdomain_boundary() {
        for trusted in [
            "https://inklura.fr",
            "https://manage.inklura.fr/path",
            "https://cloud.bext.dev",
            "https://shelldeck.1clic.pro",
        ] {
            assert!(markdown_link_is_internal(trusted), "{trusted}");
        }
        for external in [
            "https://inklura.fr.evil.example",
            "https://fakebext.dev.example",
            "https://example.com/?next=https://inklura.fr",
            "javascript:https://inklura.fr",
        ] {
            assert!(!markdown_link_is_internal(external), "{external}");
        }
    }
}
