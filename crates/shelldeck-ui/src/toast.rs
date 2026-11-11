use crate::scale::px;
use adabraka_ui::prelude::{Button, ButtonSize, ButtonVariant};
use gpui::*;
use std::rc::Rc;
use std::time::Duration;

use crate::theme::ShellDeckColors;

/// Severity level for a toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastLevel {
    /// Background color for the toast based on its level.
    fn bg_color(&self) -> Hsla {
        match self {
            ToastLevel::Info => ShellDeckColors::primary(),
            ToastLevel::Success => ShellDeckColors::success(),
            ToastLevel::Warning => ShellDeckColors::warning(),
            ToastLevel::Error => ShellDeckColors::error(),
        }
    }

    /// Text color for the toast based on its level.
    fn text_color(&self) -> Hsla {
        match self {
            // Warning uses a bright amber background -- dark text reads better
            ToastLevel::Warning => hsla(0.0, 0.0, 0.10, 1.0),
            // All others use light text on a saturated background
            _ => hsla(0.0, 0.0, 0.98, 1.0),
        }
    }

    /// Auto-dismiss duration for each level, for a short message.
    fn dismiss_duration(&self) -> Duration {
        match self {
            ToastLevel::Info | ToastLevel::Success => Duration::from_secs(3),
            ToastLevel::Warning | ToastLevel::Error => Duration::from_secs(5),
        }
    }

    /// A small prefix label rendered before the message.
    fn label(&self) -> &'static str {
        match self {
            ToastLevel::Info => "INFO",
            ToastLevel::Success => "OK",
            ToastLevel::Warning => "WARN",
            ToastLevel::Error => "ERR",
        }
    }
}

/// One answer offered by a prompt toast ([`ToastContainer::push_prompt`]).
#[derive(Clone)]
pub struct ToastAction {
    pub label: String,
    /// Runs when the answer is chosen; the toast is dismissed first.
    pub on_choose: Rc<dyn Fn(&mut App)>,
}

impl ToastAction {
    pub fn new(label: impl Into<String>, on_choose: impl Fn(&mut App) + 'static) -> Self {
        Self {
            label: label.into(),
            on_choose: Rc::new(on_choose),
        }
    }
}

/// Characters a short notice may have before it earns more time.
const SHORT_TOAST_CHARS: usize = 60;
/// Reading time added per character beyond that.
const TOAST_MILLIS_PER_CHAR: u64 = 40;
/// No notice stays longer than this by itself.
const MAX_TOAST_DURATION: Duration = Duration::from_secs(12);

/// How long a toast of `chars` characters stays before it dismisses
/// itself: the duration of its level, plus reading time for a long text.
/// A notice of two or three lines (why a terminal fell back to plain SSH,
/// why an install failed) was gone before it could be read at the fixed
/// three seconds.
fn dismiss_after(level: ToastLevel, chars: usize) -> Duration {
    let extra = chars.saturating_sub(SHORT_TOAST_CHARS) as u64 * TOAST_MILLIS_PER_CHAR;
    (level.dismiss_duration() + Duration::from_millis(extra)).min(MAX_TOAST_DURATION)
}

/// A single toast notification.
#[derive(Clone)]
struct Toast {
    id: usize,
    message: String,
    level: ToastLevel,
    /// Answers of a prompt toast. Empty for a plain notification.
    actions: Vec<ToastAction>,
}

/// Maximum number of toasts visible at once.
const MAX_VISIBLE_TOASTS: usize = 5;

/// Container that manages a queue of toast notifications.
///
/// Toasts are rendered as an overlay anchored to the bottom-right of the
/// workspace. Each toast auto-dismisses after a duration determined by its
/// severity level.
pub struct ToastContainer {
    toasts: Vec<Toast>,
    next_id: usize,
    /// We keep dismiss tasks alive so they are not cancelled on drop.
    _dismiss_tasks: Vec<gpui::Task<()>>,
}

impl Default for ToastContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastContainer {
    pub fn new() -> Self {
        Self {
            toasts: Vec::new(),
            next_id: 0,
            _dismiss_tasks: Vec::new(),
        }
    }

    /// Push a new toast notification.
    ///
    /// The toast will be automatically dismissed after a duration appropriate
    /// for its severity level. If there are already `MAX_VISIBLE_TOASTS`
    /// visible, the oldest toast is removed immediately.
    pub fn push(&mut self, message: String, level: ToastLevel, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let chars = message.chars().count();

        self.toasts.push(Toast {
            id,
            message,
            level,
            actions: Vec::new(),
        });
        self.enforce_visible_limit();

        // Spawn an auto-dismiss timer
        let duration = dismiss_after(level, chars);
        let task = cx.spawn(
            async move |this: WeakEntity<ToastContainer>, cx: &mut AsyncApp| {
                cx.background_executor().timer(duration).await;
                let _ = this.update(cx, |container, cx| {
                    container.dismiss(id, cx);
                });
            },
        );
        self._dismiss_tasks.push(task);

        cx.notify();
    }

    /// Push a question that never blocks the app: a toast with one button
    /// per answer. It stays until an answer is chosen or it is closed
    /// (closing it chooses nothing), and it is never pushed out by newer
    /// notifications. Returns its id.
    pub fn push_prompt(
        &mut self,
        message: String,
        actions: Vec<ToastAction>,
        cx: &mut Context<Self>,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.toasts.push(Toast {
            id,
            message,
            level: ToastLevel::Info,
            actions,
        });
        self.enforce_visible_limit();
        cx.notify();
        id
    }

    /// Drop the oldest plain notifications beyond the visible limit.
    /// Prompts wait for their answer and are kept.
    fn enforce_visible_limit(&mut self) {
        while self.toasts.len() > MAX_VISIBLE_TOASTS {
            let Some(oldest) = self
                .toasts
                .iter()
                .position(|toast| toast.actions.is_empty())
            else {
                break;
            };
            self.toasts.remove(oldest);
        }
    }

    /// Dismiss (remove) a toast by its id.
    pub fn dismiss(&mut self, id: usize, cx: &mut Context<Self>) {
        self.toasts.retain(|t| t.id != id);
        cx.notify();
    }

    /// Render a single toast element.
    fn render_toast(toast: &Toast, weak: WeakEntity<Self>) -> impl IntoElement {
        let id = toast.id;
        let bg = toast.level.bg_color();
        let text_col = toast.level.text_color();
        let label = toast.level.label();
        let message = toast.message.clone();

        // Message column width: toast max_w(420) minus padding, badge, gaps,
        // close affordance (~100px chrome). Per-line max_w gives gpui a
        // Definite wrap width (see `.agents/overflow.md`).
        const MSG_MAX_W: f32 = 310.0;

        let mut message_body = div()
            .flex()
            .flex_col()
            .text_size(px(13.0))
            .text_color(text_col);
        for line in message.split('\n') {
            let display: SharedString = if line.is_empty() {
                " ".into()
            } else {
                line.to_string().into()
            };
            message_body = message_body.child(div().max_w(px(MSG_MAX_W)).child(display));
        }
        if !toast.actions.is_empty() {
            // Answers of a prompt, wrapped inside the message column.
            let mut answers = div()
                .max_w(px(MSG_MAX_W))
                .pt(px(8.0))
                .flex()
                .flex_wrap()
                .gap(px(6.0));
            for (index, action) in toast.actions.iter().enumerate() {
                let on_choose = action.on_choose.clone();
                let container = weak.clone();
                answers = answers.child(
                    Button::new(
                        ElementId::Name(format!("toast-{id}-answer-{index}").into()),
                        action.label.clone(),
                    )
                    .variant(if index == 0 {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Outline
                    })
                    .size(ButtonSize::Sm)
                    .on_click(move |_, _window, cx| {
                        cx.stop_propagation();
                        if let Some(container) = container.upgrade() {
                            container.update(cx, |this, cx| this.dismiss(id, cx));
                        }
                        on_choose(cx);
                    }),
                );
            }
            message_body = message_body.child(answers);
        }

        div()
            .id(ElementId::Name(format!("toast-{}", id).into()))
            .flex()
            .items_start()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(10.0))
            .min_w(px(240.0))
            .max_w(px(420.0))
            .overflow_hidden()
            .rounded(px(8.0))
            .bg(bg)
            .shadow_lg()
            .cursor_pointer()
            .on_click(move |_event, _window, cx| {
                if let Some(container) = weak.upgrade() {
                    container.update(cx, |this, cx| {
                        this.dismiss(id, cx);
                    });
                }
            })
            .child(
                // Level badge
                div()
                    .flex_shrink_0()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(hsla(0.0, 0.0, 0.0, 0.2))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(text_col)
                            .child(label),
                    ),
            )
            .child(
                // Message text — flex_grow + min_w(0) so long content wraps
                // inside max_w instead of spilling past the toast edge.
                div()
                    .flex_grow()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(message_body),
            )
            .child(
                // Dismiss hint (x)
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(text_col.opacity(0.55))
                    .child(
                        svg()
                            .path("icons/lucide/x.svg")
                            .size(px(12.0))
                            .text_color(text_col.opacity(0.55)),
                    ),
            )
    }
}

impl Render for ToastContainer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.toasts.is_empty() {
            return div();
        }

        let weak = cx.entity().downgrade();

        let mut stack = div()
            .absolute()
            .bottom(px(40.0)) // above the status bar
            .right(px(16.0))
            .max_w(px(420.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .items_end();

        // Render from oldest (top) to newest (bottom)
        for toast in &self.toasts {
            stack = stack.child(Self::render_toast(toast, weak.clone()));
        }

        stack
    }
}

#[cfg(test)]
mod tests {
    use super::{ToastAction, ToastContainer, ToastLevel, MAX_VISIBLE_TOASTS};
    use gpui::{AppContext as _, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;

    // SDTEST-3056 (SDUC-574): a toast stays long enough to be read. A
    // short one keeps the duration of its level; a long one (why a
    // terminal fell back, why an install failed) gets reading time for
    // what exceeds a short sentence, up to a cap.
    #[test]
    fn sdtest_3056_a_long_notice_stays_longer_than_a_short_one() {
        use super::dismiss_after;
        use std::time::Duration;

        let secs = |level, chars| dismiss_after(level, chars).as_secs_f32();
        assert_eq!(secs(ToastLevel::Info, 0), 3.0);
        assert_eq!(secs(ToastLevel::Success, 60), 3.0);
        assert_eq!(secs(ToastLevel::Error, 40), 5.0);
        // The fallback notice of the session daemon, about 170 characters.
        let fallback = secs(ToastLevel::Info, 170);
        assert!((7.0..8.0).contains(&fallback), "{fallback}");
        assert!(secs(ToastLevel::Error, 170) > fallback);
        assert_eq!(
            dismiss_after(ToastLevel::Info, 100_000),
            Duration::from_secs(12)
        );
    }

    // SDTEST-3023 (SDUC-574): a question shown as a toast waits for its
    // answer: newer notifications never push it out, and closing it
    // chooses nothing.
    #[test]
    fn sdtest_3023_a_prompt_toast_waits_for_its_answer() {
        let mut app = TestAppContext::single();
        let toasts = app.update(|cx| cx.new(|_| ToastContainer::new()));
        let chosen = Rc::new(Cell::new(0u32));
        let answer = |value: u32| {
            let chosen = chosen.clone();
            ToastAction::new(format!("answer {value}"), move |_| chosen.set(value))
        };
        let prompt = toasts.update(&mut app, |toasts, cx| {
            let prompt = toasts.push_prompt("install?".into(), vec![answer(1), answer(2)], cx);
            for index in 0..MAX_VISIBLE_TOASTS + 3 {
                toasts.push(format!("notification {index}"), ToastLevel::Info, cx);
            }
            prompt
        });
        toasts.update(&mut app, |toasts, _| {
            assert_eq!(toasts.toasts.len(), MAX_VISIBLE_TOASTS);
            let kept = toasts
                .toasts
                .iter()
                .find(|toast| toast.id == prompt)
                .expect("the prompt is still shown");
            assert_eq!(kept.actions.len(), 2);
            // The newest plain notifications are the ones kept beside it.
            assert_eq!(
                toasts.toasts.last().map(|toast| toast.message.as_str()),
                Some(format!("notification {}", MAX_VISIBLE_TOASTS + 2).as_str())
            );
        });
        // Closing the prompt runs no answer.
        toasts.update(&mut app, |toasts, cx| toasts.dismiss(prompt, cx));
        assert_eq!(chosen.get(), 0);

        // Several prompts beyond the limit are all kept.
        toasts.update(&mut app, |toasts, cx| {
            for _ in 0..MAX_VISIBLE_TOASTS + 2 {
                toasts.push_prompt("install?".into(), vec![answer(3)], cx);
            }
            assert_eq!(
                toasts
                    .toasts
                    .iter()
                    .filter(|toast| !toast.actions.is_empty())
                    .count(),
                MAX_VISIBLE_TOASTS + 2
            );
        });
        // Choosing an answer runs exactly that handler.
        let handler = toasts.update(&mut app, |toasts, _| {
            toasts.toasts.last().expect("prompt").actions[0]
                .on_choose
                .clone()
        });
        app.update(|cx| handler(cx));
        assert_eq!(chosen.get(), 3);
    }
}
