//! Post-login first-run tour — skippable, replayable from Settings.
//!
//! Distinct from the pre-login welcome landing (`Workspace::render_welcome_screen`).
//! A modal that walks a new account through the surfaces of the mode it lands
//! in: one sequence per role, closed by a mode-switching slide when the account
//! can reach more than one mode, and by a shortcut strip on the last slide.
//!
//! Every slide carries artwork from `assets/images/onboarding/role-aware/`,
//! exported at 1120x400 by `scripts/export-onboarding-images.mjs` from the
//! composition in `docs/design/onboarding-role-visuals.html`. Adding one means
//! registering it in `main.rs` (`Assets::load` + `list`) as well — GPUI also
//! animates GIF natively if a slide ever needs motion.

use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::prelude::*;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::cloud_account::AppMode;

/// One slide of the tour.
///
/// The sequence is chosen from the mode the account actually lands in, not from
/// a single list shown to everybody: a client has no use for a terminal, and a
/// developer does not need to be told how to file a request. Each role gets its
/// own run, and the mode-switching slide is appended only for an account that
/// can switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardingStep {
    UserWelcome,
    UserRequest,
    UserFollow,
    UserAi,
    SupportWelcome,
    SupportPrioritize,
    SupportContext,
    SupportAi,
    SupportModes,
    DevWelcome,
    DevTerminal,
    DevScripts,
    DevTunnels,
    DevAi,
    DevModes,
}

/// A bullet of a slide: a bundled Lucide slug and the i18n suffix of its pair
/// of keys (`…_title` / `…_body`).
type Bullet = (&'static str, &'static str);

impl OnboardingStep {
    /// Role-aware artwork, exported at 1120x400 for the 560x200 media zone.
    fn media_asset(self) -> Option<&'static str> {
        Some(match self {
            Self::UserWelcome => "images/onboarding/role-aware/user-01-welcome.webp",
            Self::UserRequest => "images/onboarding/role-aware/user-02-request.webp",
            Self::UserFollow => "images/onboarding/role-aware/user-03-follow.webp",
            Self::UserAi => "images/onboarding/role-aware/user-04-ai.webp",
            Self::SupportWelcome => "images/onboarding/role-aware/support-01-welcome.webp",
            Self::SupportPrioritize => "images/onboarding/role-aware/support-02-prioritize.webp",
            Self::SupportContext => "images/onboarding/role-aware/support-03-context.webp",
            Self::SupportAi => "images/onboarding/role-aware/support-04-ai.webp",
            Self::SupportModes => "images/onboarding/role-aware/support-05-modes.webp",
            Self::DevWelcome => "images/onboarding/role-aware/dev-01-welcome.webp",
            Self::DevTerminal => "images/onboarding/role-aware/dev-02-terminal.webp",
            Self::DevScripts => "images/onboarding/role-aware/dev-03-scripts.webp",
            Self::DevTunnels => "images/onboarding/role-aware/dev-04-tunnels.webp",
            Self::DevAi => "images/onboarding/role-aware/dev-05-ai.webp",
            Self::DevModes => "images/onboarding/role-aware/dev-06-modes.webp",
        })
    }

    /// i18n prefix. Every slide owns `<prefix>.title`, `<prefix>.intro`, and one
    /// `<prefix>.<suffix>_title` / `_body` pair per bullet.
    fn key(self) -> &'static str {
        match self {
            Self::UserWelcome => "onboarding.user.welcome",
            Self::UserRequest => "onboarding.user.request",
            Self::UserFollow => "onboarding.user.follow",
            Self::UserAi => "onboarding.user.ai",
            Self::SupportWelcome => "onboarding.support.welcome",
            Self::SupportPrioritize => "onboarding.support.prioritize",
            Self::SupportContext => "onboarding.support.context",
            Self::SupportAi => "onboarding.support.ai",
            Self::SupportModes => "onboarding.support.modes",
            Self::DevWelcome => "onboarding.dev.welcome",
            Self::DevTerminal => "onboarding.dev.terminal",
            Self::DevScripts => "onboarding.dev.scripts",
            Self::DevTunnels => "onboarding.dev.tunnels",
            Self::DevAi => "onboarding.dev.ai",
            Self::DevModes => "onboarding.dev.modes",
        }
    }

    /// Icons must exist in the bundled Lucide subset (`.agents/icons.md`).
    ///
    /// A modes slide has none: it enumerates the modes this account can
    /// actually reach, which is known only at runtime.
    fn bullets(self) -> &'static [Bullet] {
        match self {
            Self::UserWelcome => &[("cloud", "sync"), ("inbox", "requests")],
            Self::UserRequest => &[("inbox", "form"), ("sparkles", "ai")],
            Self::UserFollow => &[("globe", "sites"), ("clock", "status")],
            Self::UserAi => &[("sparkles", "draft"), ("shield-check", "review")],
            Self::SupportWelcome => &[("inbox", "queue"), ("users", "contacts")],
            Self::SupportPrioritize => &[("filter", "filters"), ("user-check", "assign")],
            Self::SupportContext => &[("messages-square", "thread"), ("reply", "answer")],
            Self::SupportAi => &[("sparkles", "draft"), ("shield-check", "review")],
            Self::SupportModes => &[],
            Self::DevWelcome => &[("terminal", "ssh"), ("cloud", "sync")],
            Self::DevTerminal => &[("terminal", "sessions"), ("server", "hosts")],
            Self::DevScripts => &[("scroll-text", "library"), ("play", "run")],
            Self::DevTunnels => &[("arrow-left-right", "forward"), ("zap", "presets")],
            Self::DevAi => &[("at-sign", "mentions"), ("paperclip", "attachments")],
            Self::DevModes => &[],
        }
    }

    /// The mode-switching slide closes a run only when there is a choice to make.
    fn is_modes(self) -> bool {
        matches!(self, Self::SupportModes | Self::DevModes)
    }

    fn media_caption(self) -> String {
        let key = format!("{}.media_caption", self.key());
        t!(&key).to_string()
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
    allowed_modes: Vec<AppMode>,
    index: usize,
    focus_handle: FocusHandle,
    needs_focus: bool,
}

impl OnboardingView {
    /// `mode` is the mode the account actually lands in
    /// (`Workspace::effective_mode`), not a preference — the run is built for
    /// the surface the user is about to see.
    pub fn new(mode: AppMode, allowed_modes: &[AppMode], cx: &mut Context<Self>) -> Self {
        Self {
            steps: Self::build_steps(mode, allowed_modes),
            allowed_modes: allowed_modes.to_vec(),
            index: 0,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
        }
    }

    /// The whole run, start to finish. Kept free of `Context` so the shape of
    /// each role's tour is unit-testable without a GPUI app.
    fn build_steps(mode: AppMode, allowed_modes: &[AppMode]) -> Vec<OnboardingStep> {
        let mut steps = Self::sequence(mode);
        if allowed_modes.len() > 1 {
            // Only an account with a choice is told there is one, and the slide
            // carries the accent of the highest surface it can reach.
            steps.push(if allowed_modes.contains(&AppMode::Dev) {
                OnboardingStep::DevModes
            } else {
                OnboardingStep::SupportModes
            });
        }
        steps
    }

    fn sequence(mode: AppMode) -> Vec<OnboardingStep> {
        match mode {
            AppMode::User => vec![
                OnboardingStep::UserWelcome,
                OnboardingStep::UserRequest,
                OnboardingStep::UserFollow,
                OnboardingStep::UserAi,
            ],
            AppMode::Support => vec![
                OnboardingStep::SupportWelcome,
                OnboardingStep::SupportPrioritize,
                OnboardingStep::SupportContext,
                OnboardingStep::SupportAi,
            ],
            AppMode::Dev => vec![
                OnboardingStep::DevWelcome,
                OnboardingStep::DevTerminal,
                OnboardingStep::DevScripts,
                OnboardingStep::DevTunnels,
                OnboardingStep::DevAi,
            ],
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
            "left" | "arrowleft" if self.index > 0 => {
                self.index -= 1;
                cx.notify();
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

    /// Hero media zone. Every slide ships artwork; the placeholder branch is
    /// kept for an asset that fails to resolve, not as a design state.
    fn render_media_zone(&self, step: OnboardingStep) -> impl IntoElement {
        let caption = step.media_caption();
        let media_asset = step.media_asset();
        let has_media = media_asset.is_some();

        let mut zone = div()
            .relative()
            .w_full()
            .h(px(200.0))
            .flex_shrink_0()
            .overflow_hidden()
            .bg(ShellDeckColors::primary().opacity(0.06))
            .border_b_1()
            .border_color(ShellDeckColors::border());

        if let Some(path) = media_asset {
            zone = zone.child(img(path).w_full().h_full().object_fit(ObjectFit::Contain));
        } else {
            zone = zone.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_full()
                    .child(img("images/shelldeck-icon.png").w(px(56.0)).h(px(56.0))),
            );
        }

        let mut caption_bar = div()
            .absolute()
            .bottom(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .px(px(12.0));

        if has_media {
            caption_bar = caption_bar
                .pt(px(30.0))
                .pb(px(8.0))
                .bg(gpui::linear_gradient(
                    180.0,
                    gpui::linear_color_stop(gpui::transparent_black(), 0.0),
                    gpui::linear_color_stop(gpui::black().opacity(0.68), 1.0),
                ));
        } else {
            caption_bar = caption_bar
                .py(px(8.0))
                .bg(ShellDeckColors::bg_surface().opacity(0.92))
                .border_t_1()
                .border_color(ShellDeckColors::border());
        }

        zone.child(
            caption_bar.child(
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
            .flex_shrink_0()
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
            .py(px(5.0))
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

    /// The last slide of every run closes with the shortcuts that reach the
    /// surfaces just described — the tour used to spend a whole slide on them.
    fn render_shortcut_strip(&self) -> impl IntoElement {
        let mut strip = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .mt(px(4.0))
            .pt(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(Self::shortcut_row(
                "Ctrl+Shift+P",
                t!("onboarding.shortcuts.palette").to_string(),
            ));
        if self.allowed_modes.contains(&AppMode::Dev) {
            strip = strip
                .child(Self::shortcut_row(
                    "Ctrl+T",
                    t!("onboarding.shortcuts.terminal").to_string(),
                ))
                .child(Self::shortcut_row(
                    "Ctrl+B",
                    t!("onboarding.shortcuts.sidebar").to_string(),
                ));
        }
        strip.child(Self::shortcut_row(
            "Ctrl+,",
            t!("onboarding.shortcuts.settings").to_string(),
        ))
    }

    fn render_step_body(&self) -> impl IntoElement {
        let step = self.current();
        let key = step.key();

        let intro_key = format!("{key}.intro");
        let mut body = div().flex().flex_col().gap(px(10.0)).child(
            div()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!(&intro_key).to_string()),
        );

        if step.is_modes() {
            // The mode slide lists what this account can actually reach, so it
            // never advertises a surface the user would find missing.
            if self.allowed_modes.contains(&AppMode::User) {
                body = body.child(Self::bullet(
                    "user",
                    t!("onboarding.modes.user_title").to_string(),
                    t!("onboarding.modes.user_body").to_string(),
                ));
            }
            if self.allowed_modes.contains(&AppMode::Support) {
                body = body.child(Self::bullet(
                    "shield-check",
                    t!("onboarding.modes.support_title").to_string(),
                    t!("onboarding.modes.support_body").to_string(),
                ));
            }
            if self.allowed_modes.contains(&AppMode::Dev) {
                body = body.child(Self::bullet(
                    "cpu",
                    t!("onboarding.modes.dev_title").to_string(),
                    t!("onboarding.modes.dev_body").to_string(),
                ));
            }
        } else {
            for (icon, suffix) in step.bullets() {
                let title_key = format!("{key}.{suffix}_title");
                let body_key = format!("{key}.{suffix}_body");
                body = body.child(Self::bullet(
                    icon,
                    t!(&title_key).to_string(),
                    t!(&body_key).to_string(),
                ));
            }
        }

        if self.is_last() {
            body = body.child(self.render_shortcut_strip());
        }

        body
    }

    fn step_title(&self) -> String {
        let key = format!("{}.title", self.current().key());
        t!(&key).to_string()
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
            // Window-relative, like adabraka's `Dialog`: a fixed cap is either
            // too tall for a small window or forces the longest slide (the one
            // carrying the shortcut strip) to scroll on a roomy one.
            .max_h(relative(0.9))
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

        card = card.child(self.render_media_zone(current_step));

        card = card.child(self.render_step_dots());

        // The body is the only elastic row of the card. The last slide of a run
        // appends the shortcut strip, which pushed the total past `max_h` and
        // clipped the footer off the bottom — the exact failure
        // `.agents/overflow.md` § Centered modals describes. `flex_grow` +
        // `min_h(0)` + a scroll body is the fix it prescribes.
        card = card.child(
            div()
                .id("onboarding-body")
                .flex()
                .flex_col()
                .flex_grow()
                .min_h(px(0.0))
                .overflow_y_scroll()
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

#[cfg(test)]
mod tests {
    // Named imports, not `use super::*`: this module glob-imports `gpui`,
    // which re-exports its own `test` attribute and would shadow the built-in
    // one (`#[test]` then expands into itself until the recursion limit).
    // `sidebar.rs` and `command_palette.rs` avoid it the same way.
    use super::{OnboardingStep, OnboardingView};
    use shelldeck_core::config::cloud_account::AppMode;

    /// Every slide the tour can show. SDTEST-1665 keeps this honest against
    /// the enum.
    const ALL_STEPS: [OnboardingStep; 15] = [
        OnboardingStep::UserWelcome,
        OnboardingStep::UserRequest,
        OnboardingStep::UserFollow,
        OnboardingStep::UserAi,
        OnboardingStep::SupportWelcome,
        OnboardingStep::SupportPrioritize,
        OnboardingStep::SupportContext,
        OnboardingStep::SupportAi,
        OnboardingStep::SupportModes,
        OnboardingStep::DevWelcome,
        OnboardingStep::DevTerminal,
        OnboardingStep::DevScripts,
        OnboardingStep::DevTunnels,
        OnboardingStep::DevAi,
        OnboardingStep::DevModes,
    ];

    /// The three account tiers, exactly as `AppMode::allowed_modes` returns
    /// them for a regular user, Inklura support, and a super-admin.
    const REGULAR: &[AppMode] = &[AppMode::User];
    const SUPPORT: &[AppMode] = &[AppMode::User, AppMode::Support];
    const SUPERADMIN: &[AppMode] = &[AppMode::User, AppMode::Support, AppMode::Dev];

    fn run(mode: AppMode, allowed: &[AppMode]) -> Vec<&'static str> {
        OnboardingView::build_steps(mode, allowed)
            .iter()
            .map(|s| s.key())
            .collect()
    }

    /// SDTEST-1662 — SDUC-469.
    ///
    /// The whole decision in one table: the run follows the mode the account
    /// *lands in*, while the closing modes slide follows what the account can
    /// *reach*. The two are independent — a super-admin who left ShellDeck in
    /// User mode gets the User run and the Dev-accented modes slide — which is
    /// the pairing a per-mode-only test would miss.
    #[test]
    fn sdtest_1662_the_run_follows_the_mode_and_the_modes_slide_follows_capability() {
        let cases: [(AppMode, &[AppMode], &[&str]); 5] = [
            // A customer has one mode, so nothing tells them modes exist.
            (
                AppMode::User,
                REGULAR,
                &[
                    "onboarding.user.welcome",
                    "onboarding.user.request",
                    "onboarding.user.follow",
                    "onboarding.user.ai",
                ],
            ),
            // Inklura support on its own surface: the Support run, closed by
            // the two-mode slide.
            (
                AppMode::Support,
                SUPPORT,
                &[
                    "onboarding.support.welcome",
                    "onboarding.support.prioritize",
                    "onboarding.support.context",
                    "onboarding.support.ai",
                    "onboarding.support.modes",
                ],
            ),
            // Same account, currently in User mode: the User run, still closed
            // by the Support-accented slide.
            (
                AppMode::User,
                SUPPORT,
                &[
                    "onboarding.user.welcome",
                    "onboarding.user.request",
                    "onboarding.user.follow",
                    "onboarding.user.ai",
                    "onboarding.support.modes",
                ],
            ),
            // Platform staff on the Dev surface — the only five-slide run.
            (
                AppMode::Dev,
                SUPERADMIN,
                &[
                    "onboarding.dev.welcome",
                    "onboarding.dev.terminal",
                    "onboarding.dev.scripts",
                    "onboarding.dev.tunnels",
                    "onboarding.dev.ai",
                    "onboarding.dev.modes",
                ],
            ),
            // Same account in Support mode: Support run, Dev-accented slide.
            (
                AppMode::Support,
                SUPERADMIN,
                &[
                    "onboarding.support.welcome",
                    "onboarding.support.prioritize",
                    "onboarding.support.context",
                    "onboarding.support.ai",
                    "onboarding.dev.modes",
                ],
            ),
        ];

        for (mode, allowed, expected) in cases {
            assert_eq!(
                run(mode, allowed),
                expected,
                "run for {mode:?} / {allowed:?}"
            );
        }

        // Across every tier and landing mode: the modes slide, when present,
        // is last and appears exactly once.
        for allowed in [REGULAR, SUPPORT, SUPERADMIN] {
            for mode in [AppMode::User, AppMode::Support, AppMode::Dev] {
                let steps = OnboardingView::build_steps(mode, allowed);
                let modes = steps.iter().filter(|s| s.is_modes()).count();
                assert_eq!(
                    modes,
                    usize::from(allowed.len() > 1),
                    "modes slide count for {mode:?} / {allowed:?}",
                );
                if modes == 1 {
                    assert!(
                        steps.last().expect("non-empty run").is_modes(),
                        "{mode:?} / {allowed:?} must close on the modes slide",
                    );
                }
            }
        }
    }

    /// SDTEST-1663 — SDUC-469.
    ///
    /// Every slide must resolve the copy it asks for. `rust_i18n` renders a
    /// missing key as the key itself, so a typo in `key()`/`bullets()` or a
    /// forgotten line in `fr.toml` ships the literal
    /// `onboarding.dev.tunnels.presets_body` to the user instead of failing.
    ///
    /// Deliberately does not call `set_locale` — that is process-global and
    /// races the rest of the suite (see `i18n::tests::locale_fr_and_en`).
    /// Resolving under the ambient locale proves the key exists; SDTEST-1302
    /// already proves `fr.toml` and `en.toml` declare the same keys.
    #[test]
    fn sdtest_1663_every_slide_resolves_its_copy() {
        for step in ALL_STEPS {
            let prefix = step.key();
            let mut keys = vec![
                format!("{prefix}.title"),
                format!("{prefix}.intro"),
                format!("{prefix}.media_caption"),
            ];
            for (_, suffix) in step.bullets() {
                keys.push(format!("{prefix}.{suffix}_title"));
                keys.push(format!("{prefix}.{suffix}_body"));
            }
            for key in keys {
                let shown = crate::t!(&key).to_string();
                assert_ne!(shown, key, "{key} has no translation");
                assert!(!shown.trim().is_empty(), "{key} resolves to blank");
            }
        }

        // The modes slide borrows the shared mode descriptions rather than
        // declaring bullets of its own, and the last slide of every run ends
        // on the shortcut strip.
        for key in [
            "onboarding.modes.user_title",
            "onboarding.modes.user_body",
            "onboarding.modes.support_title",
            "onboarding.modes.support_body",
            "onboarding.modes.dev_title",
            "onboarding.modes.dev_body",
            "onboarding.shortcuts.palette",
            "onboarding.shortcuts.terminal",
            "onboarding.shortcuts.sidebar",
            "onboarding.shortcuts.settings",
        ] {
            assert_ne!(crate::t!(key).to_string(), key, "{key} has no translation");
        }
    }

    /// SDTEST-1664 — SDUC-469.
    ///
    /// A slide's artwork renders only if `main.rs` both embeds it
    /// (`include_bytes!`) and lists it (`Assets::list`). Neither is checked by
    /// the compiler: an unregistered path makes the image load fail silently,
    /// leaving an empty hero zone that nothing else would catch.
    #[test]
    fn sdtest_1664_every_slide_asset_is_embedded_and_listed() {
        let main_rs = include_str!("../../shelldeck/src/main.rs");
        for step in ALL_STEPS {
            let asset = step.media_asset().expect("every slide ships artwork");
            assert!(
                main_rs.contains(&format!("\"{asset}\" =>")),
                "{asset} is not embedded in main.rs (Assets::load)",
            );
            assert!(
                main_rs.contains(&format!("SharedString::from(\"{asset}\")")),
                "{asset} is missing from Assets::list",
            );
        }
    }

    /// SDTEST-1665 — SDUC-469.
    ///
    /// `ALL_STEPS` drives the two tests above, so it must not drift from the
    /// enum: the union of every reachable run has to account for all fifteen
    /// slides, and no slide may be unreachable.
    #[test]
    fn sdtest_1665_runs_cover_every_slide() {
        let mut seen: Vec<OnboardingStep> = Vec::new();
        for allowed in [REGULAR, SUPPORT, SUPERADMIN] {
            for mode in [AppMode::User, AppMode::Support, AppMode::Dev] {
                for step in OnboardingView::build_steps(mode, allowed) {
                    if !seen.contains(&step) {
                        seen.push(step);
                    }
                }
            }
        }
        assert_eq!(
            seen.len(),
            ALL_STEPS.len(),
            "a slide is unreachable or ALL_STEPS is stale",
        );
        for step in ALL_STEPS {
            assert!(seen.contains(&step), "{step:?} is never shown to anyone");
        }
    }
}
