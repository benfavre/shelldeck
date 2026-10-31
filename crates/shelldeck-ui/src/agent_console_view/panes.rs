//! Independent repaint scopes keep transcript motion from rebuilding the navigator.
use super::*;

pub(super) struct NavigatorPane {
    parent: WeakEntity<AgentConsoleView>,
    _subscription: Subscription,
}
impl NavigatorPane {
    pub(super) fn new(parent: WeakEntity<AgentConsoleView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(
            &parent.upgrade().expect("constructing console"),
            |_, _, cx| cx.notify(),
        );
        Self {
            parent,
            _subscription: subscription,
        }
    }
}
impl Render for NavigatorPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.parent
            .update(cx, |parent, cx| parent.render_navigator(cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

pub(super) struct TimelinePane {
    parent: WeakEntity<AgentConsoleView>,
    _subscription: Subscription,
}
impl TimelinePane {
    pub(super) fn new(parent: WeakEntity<AgentConsoleView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(
            &parent.upgrade().expect("constructing console"),
            |_, _, cx| cx.notify(),
        );
        Self {
            parent,
            _subscription: subscription,
        }
    }
}
impl Render for TimelinePane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.parent
            .update(cx, |parent, cx| {
                parent.render_virtual_timeline(parent.tools.navigator_wide, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}
