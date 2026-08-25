use std::time::Duration;

use gpui::prelude::*;
use gpui::*;

use crate::brand::brand_badge;
use crate::motion::repeating_phase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MonolithMotion {
    FloatAndBreathe,
    UserCompanion,
    SupportScan,
    DevTyping,
    Thinking,
    SiteScan,
    TerminalTyping,
}

/// Render a complete Monolith expression under the shared motion policy.
///
/// Authored motions are transparent lossless animated WebP files exported from
/// the browser motion lab. The SVG post-login expression derives its transform
/// from the shared clock, and every motion falls back to a static asset when
/// the platform requests reduced motion.
pub(crate) fn animated_monolith<V: 'static>(
    id: &'static str,
    size: f32,
    motion: MonolithMotion,
    cx: &mut Context<V>,
) -> AnyElement {
    let reduced_motion = cx.prefers_reduced_motion();
    let root = div().relative().flex_shrink_0().w(px(size)).h(px(size));

    if motion == MonolithMotion::FloatAndBreathe {
        if reduced_motion {
            return root
                .child(
                    img("images/brand/svg/expressions/dark-default-logo.svg")
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element();
        }
        let phase = repeating_phase(Duration::from_millis(2_500), cx);
        let wave = (phase * std::f32::consts::TAU).sin();
        return root
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        img("images/brand/svg/expressions/dark-default-logo.svg")
                            .w_full()
                            .h_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .top(px(wave * 4.0))
                    .scale(0.99 + (wave + 1.0) * 0.005),
            )
            .into_any_element();
    }

    if reduced_motion {
        return brand_badge(size).into_any_element();
    }

    let asset = match motion {
        MonolithMotion::FloatAndBreathe => unreachable!(),
        MonolithMotion::UserCompanion => "images/brand/webp/modes/monolith-user.webp",
        MonolithMotion::SupportScan => "images/brand/webp/modes/monolith-support.webp",
        MonolithMotion::DevTyping => "images/brand/webp/modes/monolith-dev.webp",
        MonolithMotion::Thinking => "images/brand/webp/studies/monolith-thinking.webp",
        MonolithMotion::SiteScan => "images/brand/webp/studies/monolith-scan.webp",
        MonolithMotion::TerminalTyping => "images/brand/webp/studies/monolith-terminal-typing.webp",
    };

    root.child(
        img(asset)
            .id(SharedString::from(format!("{id}-animated")))
            .w_full()
            .h_full()
            .object_fit(ObjectFit::Contain),
    )
    .into_any_element()
}

/// Keep the loading label stable while cycling through "", ".", "..", "...".
///
/// The suffix owns a fixed width so adjacent controls do not shift whenever a
/// dot is added or removed.
pub(crate) fn animated_loading_text<V: 'static>(
    id: &'static str,
    label: impl Into<SharedString>,
    cx: &mut Context<V>,
) -> AnyElement {
    let label = label.into();
    let phase = repeating_phase(Duration::from_millis(1_200), cx);
    let dots = match phase {
        value if value < 0.25 => "",
        value if value < 0.50 => ".",
        value if value < 0.75 => "..",
        _ => "...",
    };

    div()
        .flex()
        .items_center()
        .child(label)
        .child(
            div()
                .id(SharedString::from(format!("{id}-ellipsis")))
                .w(px(18.0))
                .flex_shrink_0()
                .child(dots),
        )
        .into_any_element()
}
