use gpui::*;
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

    /// Auto-dismiss duration for each level.
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

/// A single toast notification.
#[derive(Debug, Clone)]
struct Toast {
    id: usize,
    message: String,
    level: ToastLevel,
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

        self.toasts.push(Toast { id, message, level });

        // Enforce the maximum visible count by dropping the oldest
        while self.toasts.len() > MAX_VISIBLE_TOASTS {
            self.toasts.remove(0);
        }

        // Spawn an auto-dismiss timer
        let duration = level.dismiss_duration();
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

        div()
            .id(ElementId::Name(format!("toast-{}", id).into()))
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(14.0))
            .py(px(10.0))
            .min_w(px(240.0))
            .max_w(px(420.0))
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
                // Message text
                div()
                    .flex_grow()
                    .text_size(px(13.0))
                    .text_color(text_col)
                    .child(message),
            )
            .child(
                // Dismiss hint (x)
                div()
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .text_color(hsla(0.0, 0.0, 1.0, 0.5))
                    .child("\u{00D7}"), // multiplication sign as x
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
