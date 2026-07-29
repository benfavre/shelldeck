use std::time::Duration;

use gpui::prelude::*;
use gpui::*;

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

/// Render a complete Monolith expression with one stable animation clock.
///
/// The authored motions are transparent lossless animated WebP files exported
/// from the browser motion lab. The post-login splash remains a lightweight
/// GPUI animation because its source is the canonical full-expression SVG.
pub(crate) fn animated_monolith(
    id: &'static str,
    size: f32,
    motion: MonolithMotion,
) -> impl IntoElement {
    let root = div().relative().flex_shrink_0().w(px(size)).h(px(size));

    if motion == MonolithMotion::FloatAndBreathe {
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
                    .with_animation(
                        SharedString::from(format!("{id}-breathe")),
                        Animation::new(Duration::from_millis(2_500))
                            .repeat()
                            .with_easing(ease_in_out),
                        |element, delta| {
                            let wave = (delta * std::f32::consts::TAU).sin();
                            element
                                .top(px(wave * 4.0))
                                .scale(0.99 + (wave + 1.0) * 0.005)
                        },
                    ),
            )
            .into_any_element();
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
pub(crate) fn animated_loading_text(
    id: &'static str,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    let label = label.into();

    div().flex().items_center().child(label).child(
        div().w(px(18.0)).flex_shrink_0().with_animation(
            SharedString::from(format!("{id}-ellipsis")),
            Animation::new(Duration::from_millis(1_200)).repeat(),
            |element, delta| {
                let dots = match delta {
                    value if value < 0.25 => "",
                    value if value < 0.50 => ".",
                    value if value < 0.75 => "..",
                    _ => "...",
                };
                element.child(dots)
            },
        ),
    )
}
