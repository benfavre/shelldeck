//! Lucide + Simple Icons helpers.
//!
//! - Lucide: `crates/shelldeck/assets/icons/lucide/` (see README there)
//! - Simple Icons: `crates/shelldeck/assets/icons/simple/` (tech / brand marks
//!   from https://github.com/LitoMore/simple-icons-cdn — SVGs embed brand hex)

use adabraka_ui::components::icon::{Icon, IconSize};
use gpui::{prelude::*, SharedString, *};
use shelldeck_core::ai::AiBackend;
use shelldeck_core::models::script::{ScriptCategory, ScriptLanguage};

use crate::theme::ShellDeckColors;

/// GPUI asset path for `svg().path(lucide_path("x"))` — inherits parent
/// `text_color` (handy for hover states on a wrapping div).
pub fn lucide_path(name: &str) -> SharedString {
    SharedString::from(format!("icons/lucide/{name}.svg"))
}

/// GPUI asset path for Simple Icons (`icons/simple/{slug}.svg`).
pub fn simple_path(name: &str) -> SharedString {
    SharedString::from(format!("icons/simple/{name}.svg"))
}

fn language_brand_hsla(lang: &ScriptLanguage) -> Hsla {
    let (r, g, b) = lang.badge_color();
    hsla(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

/// Language mark at official brand color. GPUI SVG paint **requires** `text_color`
/// (`svg.rs` skips render when `style.text.color` is None) — set it on the
/// icon node only, never on the chip wrapper (parent tint washes selected state).
pub fn script_language_icon(lang: ScriptLanguage, size_px: f32) -> impl IntoElement {
    svg()
        .path(simple_path(lang.simple_icon()))
        .size(px(size_px))
        .flex_shrink_0()
        .text_color(language_brand_hsla(&lang))
}

/// Render a named Lucide icon at `size_px` logical pixels, tinted with `color`.
pub fn lucide_icon(name: &str, size_px: f32, color: Hsla) -> impl IntoElement {
    Icon::new(name)
        .size(IconSize::Custom(px(size_px)))
        .color(color)
}

/// Simple Icons mark tinted with `color` (mono via GPUI — prefer embedded-fill
/// SVGs + `script_language_icon` for brand marks).
pub fn simple_icon(name: &str, size_px: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(simple_path(name))
        .size(px(size_px))
        .flex_shrink_0()
        .text_color(color)
}

/// Compact, non-interactive provider signature shared by every AI surface.
pub fn ai_provider_badge(backend: AiBackend, model: &str) -> impl IntoElement {
    let icon = match backend {
        AiBackend::ClaudeCli => {
            simple_icon("claudecode", 14.0, ShellDeckColors::text_primary()).into_any_element()
        }
        AiBackend::CodexCli | AiBackend::OpenAi => {
            simple_icon("openai", 14.0, ShellDeckColors::text_primary()).into_any_element()
        }
        AiBackend::Anthropic => {
            simple_icon("anthropic", 14.0, ShellDeckColors::text_primary()).into_any_element()
        }
        AiBackend::AiderCli => {
            lucide_icon("terminal", 14.0, ShellDeckColors::text_primary()).into_any_element()
        }
        AiBackend::Disabled => {
            lucide_icon("sparkles", 14.0, ShellDeckColors::text_muted()).into_any_element()
        }
    };
    let model = model.trim().to_string();

    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(6.0))
        .h(px(30.0))
        .max_w(px(190.0))
        .px(px(9.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(ShellDeckColors::primary().opacity(0.28))
        .text_size(px(11.0))
        .text_color(ShellDeckColors::text_primary())
        .child(icon)
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(if model.is_empty() {
                    backend.display_name().to_string()
                } else {
                    format!("{} · {}", backend.display_name(), model)
                }),
        )
}

/// Chip row: brand SVG icon + label (icon keeps embedded fill).
pub fn script_language_chip(lang: ScriptLanguage, label_color: Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(
            div()
                .flex_shrink_0()
                .child(script_language_icon(lang.clone(), 12.0)),
        )
        .child(
            div()
                .text_color(label_color)
                .child(lang.label().to_string()),
        )
}

/// Chip row: Lucide icon + label for a script category.
pub fn script_category_chip(cat: ScriptCategory, color: Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(lucide_icon(cat.lucide_icon(), 12.0, color))
        .child(cat.label().to_string())
}
