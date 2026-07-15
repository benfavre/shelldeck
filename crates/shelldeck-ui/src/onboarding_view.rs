//! Post-login first-run tour — skippable, replayable from Settings.
//!
//! Distinct from the pre-login welcome landing (`Workspace::render_welcome_screen`).
//! A multi-step modal that orients new users on modes, surfaces, and shortcuts.
//!
//! Each step has a hero media **slot** (styled placeholder by default). To add a
//! GIF/WebP for one step only: drop the file under `assets/images/onboarding/`,
//! register it in `main.rs` (`Assets::load` + `list`), then set `media_asset()`
//! to `Some("images/onboarding/…")` for that step — GPUI animates GIF natively.

use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::prelude::*;
use gpui::prelude::*;
use gpui::*;

/// Which slide is shown. `Modes` is omitted when the signed-in user cannot
/// switch modes (forced User).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardingStep {
    Welcome,
    Modes,
    Surfaces,
    Shortcuts,
}

impl OnboardingStep {
    /// Optional embedded media (GIF/WebP/PNG). `None` → styled placeholder slot.
    fn media_asset(self) -> Option<&'static str> {
        match self {
            // Per-step — enable when the asset exists + is wired in main.rs:
            // Self::Welcome => Some("images/onboarding/welcome.gif"),
            // Self::Modes => Some("images/onboarding/modes.gif"),
            // Self::Surfaces => Some("images/onboarding/surfaces.gif"),
            // Self::Shortcuts => Some("images/onboarding/shortcuts.gif"),
            _ => None,
        }
    }

    fn placeholder_icon(self) -> &'static str {
        match self {
            Self::Welcome => "terminal",
            Self::Modes => "grid-2x2",
            Self::Surfaces => "globe",
            Self::Shortcuts => "table",
        }
    }

    fn media_caption(self) -> String {
        match self {
            Self::Welcome => t!("onboarding.welcome.media_caption").to_string(),
            Self::Modes => t!("onboarding.modes.media_caption").to_string(),
            Self::Surfaces => t!("onboarding.surfaces.media_caption").to_string(),
            Self::Shortcuts => t!("onboarding.shortcuts.media_caption").to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum OnboardingEvent {
    /// User finished the last step — persist `onboarding_completed`.
    Finished,
    /// User skipped or closed — still persist so we don't nag again.
    Skipped,
}

impl EventEmitter<OnboardingEvent> for OnboardingView {}

pub struct OnboardingView {
    steps: Vec<OnboardingStep>,
    index: usize,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

impl OnboardingView {
    pub fn new(can_switch_mode: bool, cx: &mut Context<Self>) -> Self {
        let mut steps = vec![OnboardingStep::Welcome];
        if can_switch_mode {
            steps.push(OnboardingStep::Modes);
        }
        steps.push(OnboardingStep::Surfaces);
        steps.push(OnboardingStep::Shortcuts);
        Self {
            steps,
            index: 0,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    fn current(&self) -> OnboardingStep {
        self.steps[self.index]
    }

    fn is_last(&self) -> bool {
        self.index + 1 >= self.steps.len()
    }

    fn step_label(&self) -> String {
        t!(
            "onboarding.step_counter",
            current = (self.index + 1),
            total = self.steps.len()
        )
        .to_string()
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(OnboardingEvent::Skipped),
            "enter" => {
                if self.is_last() {
                    cx.emit(OnboardingEvent::Finished);
                } else {
                    self.index += 1;
                    cx.notify();
                }
            }
            "left" | "arrowleft" => {
                if self.index > 0 {
                    self.index -= 1;
                    cx.notify();
                }
            }
            "right" | "arrowright" => {
                if self.is_last() {
                    cx.emit(OnboardingEvent::Finished);
                } else {
                    self.index += 1;
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// Hero media slot — placeholder until `media_asset()` is set for a step.
    fn render_media_zone(step: OnboardingStep) -> impl IntoElement {
        let caption = step.media_caption();
        let has_media = step.media_asset().is_some();

        let mut zone = div()
            .relative()
            .w_full()
            .h(px(200.0))
            .flex_shrink_0()
            .overflow_hidden()
            .bg(ShellDeckColors::primary().opacity(0.06))
            .border_b_1()
            .border_color(ShellDeckColors::border());

        if let Some(path) = step.media_asset() {
            zone = zone.child(img(path).w_full().h_full().object_fit(ObjectFit::Cover));
        } else {
            let mut inner = div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .size_full()
                .px(px(20.0));

            if step == OnboardingStep::Welcome {
                inner = inner.child(img("images/shelldeck-icon.png").w(px(56.0)).h(px(56.0)));
            } else {
                inner = inner.child(lucide_icon(
                    step.placeholder_icon(),
                    40.0,
                    ShellDeckColors::primary().opacity(0.55),
                ));
            }

            inner = inner.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("onboarding.media_soon").to_string()),
            );

            zone = zone.child(inner);
        }

        zone.child(
            div()
                .absolute()
                .bottom(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .px(px(12.0))
                .py(px(8.0))
                .bg(if has_media {
                    gpui::black().opacity(0.45)
                } else {
                    ShellDeckColors::bg_surface().opacity(0.92)
                })
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if has_media {
                            gpui::white().opacity(0.92)
                        } else {
                            ShellDeckColors::text_muted()
                        })
                        .child(caption),
                ),
        )
    }

    fn render_step_dots(&self) -> impl IntoElement {
        let mut row = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .py(px(10.0));
        for (i, _) in self.steps.iter().enumerate() {
            let active = i == self.index;
            row = row.child(
                div()
                    .h(px(6.0))
                    .w(px(if active { 18.0 } else { 6.0 }))
                    .rounded(px(3.0))
                    .bg(if active {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::border()
                    }),
            );
        }
        row
    }

    fn bullet(icon: &'static str, title: String, body: String) -> impl IntoElement {
        div()
            .flex()
            .gap(px(10.0))
            .child(div().flex_shrink_0().mt(px(2.0)).child(lucide_icon(
                icon,
                16.0,
                ShellDeckColors::primary(),
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(body),
                    ),
            )
    }

    fn shortcut_row(keys: &str, desc: String) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .py(px(6.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(desc),
            )
            .child(
                div()
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_sidebar())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .child(keys.to_string()),
            )
    }

    fn render_step_body(&self) -> impl IntoElement {
        match self.current() {
            OnboardingStep::Welcome => div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("onboarding.welcome.body").to_string()),
                )
                .child(Self::bullet(
                    "cloud",
                    t!("onboarding.welcome.bullet_sync_title").to_string(),
                    t!("onboarding.welcome.bullet_sync_body").to_string(),
                ))
                .child(Self::bullet(
                    "terminal",
                    t!("onboarding.welcome.bullet_ssh_title").to_string(),
                    t!("onboarding.welcome.bullet_ssh_body").to_string(),
                )),
            OnboardingStep::Modes => div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("onboarding.modes.intro").to_string()),
                )
                .child(Self::bullet(
                    "user",
                    t!("onboarding.modes.user_title").to_string(),
                    t!("onboarding.modes.user_body").to_string(),
                ))
                .child(Self::bullet(
                    "shield-check",
                    t!("onboarding.modes.support_title").to_string(),
                    t!("onboarding.modes.support_body").to_string(),
                ))
                .child(Self::bullet(
                    "cpu",
                    t!("onboarding.modes.dev_title").to_string(),
                    t!("onboarding.modes.dev_body").to_string(),
                )),
            OnboardingStep::Surfaces => div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("onboarding.surfaces.intro").to_string()),
                )
                .child(Self::bullet(
                    "globe",
                    t!("onboarding.surfaces.sites_title").to_string(),
                    t!("onboarding.surfaces.sites_body").to_string(),
                ))
                .child(Self::bullet(
                    "inbox",
                    t!("onboarding.surfaces.requests_title").to_string(),
                    t!("onboarding.surfaces.requests_body").to_string(),
                ))
                .child(Self::bullet(
                    "search",
                    t!("onboarding.surfaces.palette_title").to_string(),
                    t!("onboarding.surfaces.palette_body").to_string(),
                )),
            OnboardingStep::Shortcuts => div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .mb(px(8.0))
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("onboarding.shortcuts.intro").to_string()),
                )
                .child(Self::shortcut_row(
                    "Ctrl+Shift+P",
                    t!("onboarding.shortcuts.palette").to_string(),
                ))
                .child(Self::shortcut_row(
                    "Ctrl+T",
                    t!("onboarding.shortcuts.terminal").to_string(),
                ))
                .child(Self::shortcut_row(
                    "Ctrl+B",
                    t!("onboarding.shortcuts.sidebar").to_string(),
                ))
                .child(Self::shortcut_row(
                    "Ctrl+,",
                    t!("onboarding.shortcuts.settings").to_string(),
                )),
        }
    }

    fn step_title(&self) -> String {
        match self.current() {
            OnboardingStep::Welcome => t!("onboarding.welcome.title").to_string(),
            OnboardingStep::Modes => t!("onboarding.modes.title").to_string(),
            OnboardingStep::Surfaces => t!("onboarding.surfaces.title").to_string(),
            OnboardingStep::Shortcuts => t!("onboarding.shortcuts.title").to_string(),
        }
    }
}

impl Render for OnboardingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            self.focus_handle.focus(window);
        }

        let entity = cx.entity();
        let is_last = self.is_last();
        let is_first = self.index == 0;
        let step_label = self.step_label();
        let step_title = self.step_title();
        let current_step = self.current();

        let mut card = div()
            .flex()
            .flex_col()
            .w(px(560.0))
            .max_h(px(600.0))
            .bg(ShellDeckColors::bg_surface())
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .shadow_xl()
            .overflow_hidden();

        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(20.0))
                .py(px(16.0))
                .flex_shrink_0()
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(step_label),
                        )
                        .child(
                            div()
                                .text_size(px(17.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(step_title),
                        ),
                )
                .child(
                    div()
                        .id("onboarding-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_color(ShellDeckColors::text_muted())
                        .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                        .child(
                            svg()
                                .path("icons/lucide/x.svg")
                                .size(px(14.0))
                                .text_color(ShellDeckColors::text_muted()),
                        )
                        .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                            cx.emit(OnboardingEvent::Skipped);
                        })),
                ),
        );

        card = card.child(Self::render_media_zone(current_step));

        card = card.child(self.render_step_dots());

        card = card.child(
            div()
                .px(px(20.0))
                .pb(px(16.0))
                .child(self.render_step_body()),
        );

        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(20.0))
                .py(px(14.0))
                .flex_shrink_0()
                .border_t_1()
                .border_color(ShellDeckColors::border())
                .child(
                    Button::new("onboarding-skip", t!("onboarding.skip").to_string())
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                            cx.emit(OnboardingEvent::Skipped);
                        })),
                )
                .child({
                    let mut row = div().flex().gap(px(8.0));
                    if !is_first {
                        row = row.child(
                            Button::new("onboarding-prev", t!("onboarding.prev").to_string())
                                .variant(ButtonVariant::Outline)
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            if this.index > 0 {
                                                this.index -= 1;
                                                cx.notify();
                                            }
                                        });
                                    }
                                }),
                        );
                    }
                    if is_last {
                        row = row.child(
                            Button::new("onboarding-finish", t!("onboarding.finish").to_string())
                                .variant(ButtonVariant::Default)
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(OnboardingEvent::Finished);
                                })),
                        );
                    } else {
                        row = row.child(
                            Button::new("onboarding-next", t!("onboarding.next").to_string())
                                .variant(ButtonVariant::Default)
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.index += 1;
                                            cx.notify();
                                        });
                                    }
                                }),
                        );
                    }
                    row
                }),
        );

        div()
            .id("onboarding-overlay")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key_down(event, cx);
            }))
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(ShellDeckColors::backdrop())
            .flex()
            .items_center()
            .justify_center()
            .child(card)
    }
}
