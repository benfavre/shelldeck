use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MonolithMotion {
    FloatAndBreathe,
    UserCompanion,
    SupportScan,
    DevTyping,
}

/// Renders the complete Monolith expression for splash and mode transitions.
///
/// The mode personalities are authored in the browser motion lab, exported as
/// transparent lossless animated WebP files, and decoded natively by GPUI.
/// Keeping the full motion in one image gives it one stable frame clock rather
/// than several independently scheduled GPUI animation elements.
pub(super) fn animated_monolith(
    id: &'static str,
    size: f32,
    motion: MonolithMotion,
) -> impl IntoElement {
    use std::time::Duration;

    let root = div()
        .relative()
        .flex_shrink_0()
        .w(px(size))
        .h(px(size));

    match motion {
        MonolithMotion::FloatAndBreathe => root.child(
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
        ),
        MonolithMotion::UserCompanion
        | MonolithMotion::SupportScan
        | MonolithMotion::DevTyping => {
            let asset = match motion {
                MonolithMotion::UserCompanion => {
                    "images/brand/webp/modes/monolith-user.webp"
                }
                MonolithMotion::SupportScan => {
                    "images/brand/webp/modes/monolith-support.webp"
                }
                MonolithMotion::DevTyping => {
                    "images/brand/webp/modes/monolith-dev.webp"
                }
                MonolithMotion::FloatAndBreathe => unreachable!(),
            };

            root.child(
                img(asset)
                    .id(SharedString::from(format!("{id}-animated")))
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Contain),
            )
        }
    }
}
