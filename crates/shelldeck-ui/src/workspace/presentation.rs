//! Transient workspace presentation: the full app or one focused Dev view.
//! Authentication and the persisted app mode remain authoritative.
use super::*;
use crate::shortcut_reference::BindableAction;
use crate::view_attention::ViewAttention;

/// What the window shows around the Dev views: the full app (titlebar menus,
/// activity rail, every destination) or one focused view with no rail.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspacePresentation {
    #[default]
    Main,
    Agents,
    Terminal,
    Editor,
}

impl WorkspacePresentation {
    /// Switcher order, which is also the order the cycle shortcut follows.
    pub const ALL: [Self; 4] = [Self::Main, Self::Agents, Self::Terminal, Self::Editor];

    pub fn is_focused(self) -> bool {
        self != Self::Main
    }

    /// The one Dev view a focused presentation shows.
    pub(super) fn view(self) -> Option<ActiveView> {
        match self {
            Self::Main => None,
            Self::Agents => Some(ActiveView::Agents),
            Self::Terminal => Some(ActiveView::Terminal),
            Self::Editor => Some(ActiveView::FileEditor),
        }
    }

    fn section(self) -> Option<SidebarSection> {
        match self {
            Self::Main => None,
            Self::Agents => Some(SidebarSection::Agents),
            Self::Terminal => Some(SidebarSection::Terminals),
            Self::Editor => Some(SidebarSection::FileEditor),
        }
    }

    /// The focused presentation that shows `view`, if there is one.
    fn for_view(view: ActiveView) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|presentation| presentation.view() == Some(view))
    }

    /// Where a focused window goes when its Dev view changes: it stays
    /// focused when the destination has a focused presentation of its own
    /// and opens the full app for every other destination.
    pub(super) fn after_view_change(self, view: ActiveView) -> Self {
        if !self.is_focused() {
            return self;
        }
        Self::for_view(view).unwrap_or(Self::Main)
    }

    /// Whether `ToggleSidebar` has something to act on: the sidebar panel of
    /// the full app or the session navigator of Agents. The focused Terminal
    /// and Editor views have neither.
    pub(super) fn has_sidebar_toggle(self) -> bool {
        matches!(self, Self::Main | Self::Agents)
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|presentation| *presentation == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> String {
        t!(match self {
            Self::Main => "presentation.full",
            Self::Agents => "presentation.agents",
            Self::Terminal => "presentation.terminal",
            Self::Editor => "presentation.editor",
        })
        .to_string()
    }

    /// Lucide slug from the bundled subset (`.agents/icons.md`).
    pub fn icon(self) -> &'static str {
        match self {
            Self::Main => "layout-dashboard",
            Self::Agents => "bot",
            Self::Terminal => "terminal",
            Self::Editor => "pencil",
        }
    }

    pub fn window_title(self) -> &'static str {
        match self {
            Self::Main => shelldeck_core::APP_NAME,
            Self::Agents => shelldeck_core::AGENTS_APP_NAME,
            Self::Terminal => shelldeck_core::TERMINAL_APP_NAME,
            Self::Editor => shelldeck_core::EDITOR_APP_NAME,
        }
    }

    fn switcher_id(self) -> &'static str {
        match self {
            Self::Main => "presentation-switch-full",
            Self::Agents => "presentation-switch-agents",
            Self::Terminal => "presentation-switch-terminal",
            Self::Editor => "presentation-switch-editor",
        }
    }

    fn after_auth(self, signed_in: bool, dev_allowed: bool) -> Self {
        if signed_in && !dev_allowed {
            Self::Main
        } else {
            self
        }
    }

    pub(super) fn effective_mode(self, dev_allowed: bool, resolved: AppMode) -> AppMode {
        if self.is_focused() && dev_allowed {
            AppMode::Dev
        } else {
            resolved
        }
    }
}

/// The dot of each switcher segment, in [`WorkspacePresentation::ALL`]
/// order. A view that is on screen asks for nothing there: its own surface
/// already shows it. The full app and the editor report nothing.
fn switcher_attention(
    on_screen: Option<ActiveView>,
    agents: ViewAttention,
    terminal: ViewAttention,
) -> [ViewAttention; 4] {
    WorkspacePresentation::ALL.map(|presentation| {
        if on_screen.is_some() && presentation.view() == on_screen {
            return ViewAttention::None;
        }
        match presentation {
            WorkspacePresentation::Agents => agents,
            WorkspacePresentation::Terminal => terminal,
            WorkspacePresentation::Main | WorkspacePresentation::Editor => ViewAttention::None,
        }
    })
}

pub(super) fn agent_surface_visible(
    dev_allowed: bool,
    mode: AppMode,
    settings_open: bool,
    agents_selected: bool,
) -> bool {
    dev_allowed && mode == AppMode::Dev && !settings_open && agents_selected
}

impl Workspace {
    /// Request a focused presentation, including before login. The welcome
    /// screen and the server-issued Dev capability still gate entry.
    /// `Main` restores the full app on its Agents landing.
    pub fn request_presentation(
        &mut self,
        presentation: WorkspacePresentation,
        cx: &mut Context<Self>,
    ) {
        if !presentation.is_focused() {
            self.show_main_presentation(cx);
            return;
        }
        self.presentation = presentation;
        self.presentation_focus_pending = true;
        self.settings_open = false;
        self.settings_dialog = None;
        self.mode_transition = None;
        self.close_titlebar_menus();
        self.activate_current_mode(cx);
        cx.notify();
    }

    /// Restore the full shell on Agents without changing the saved app mode.
    pub fn show_main_presentation(&mut self, cx: &mut Context<Self>) {
        self.presentation = WorkspacePresentation::Main;
        self.presentation_focus_pending = false;
        self.settings_open = false;
        self.settings_dialog = None;
        self.mode_transition = None;
        self.close_titlebar_menus();
        self.select_presentation_landing(WorkspacePresentation::Agents, cx);
        self.activate_current_mode(cx);
        cx.notify();
    }

    /// The user picked a presentation (titlebar switcher, palette, View menu,
    /// cycle shortcut). Unlike a launch request, returning to the full app
    /// keeps the Dev view that was selected, so with Dev as the saved mode
    /// the terminal or the file being edited stays in front with the rail
    /// around it. Another saved mode (User, Support) comes back as itself.
    pub fn set_presentation(
        &mut self,
        presentation: WorkspacePresentation,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        if presentation.is_focused() {
            self.request_presentation(presentation, cx);
            return;
        }
        if !self.presentation.is_focused() {
            return;
        }
        self.presentation = WorkspacePresentation::Main;
        self.presentation_focus_pending = false;
        self.settings_open = false;
        self.settings_dialog = None;
        self.mode_transition = None;
        self.close_titlebar_menus();
        self.activate_current_mode(cx);
        cx.notify();
    }

    pub fn cycle_presentation(&mut self, cx: &mut Context<Self>) {
        self.set_presentation(self.active_presentation().next(), cx);
    }

    fn select_presentation_landing(
        &mut self,
        presentation: WorkspacePresentation,
        cx: &mut Context<Self>,
    ) {
        let (Some(view), Some(section)) = (presentation.view(), presentation.section()) else {
            return;
        };
        self.active_view = view;
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active_section(section);
            cx.notify();
        });
        if !(self.can_access_mode(AppMode::Dev) && self.effective_mode() == AppMode::Dev) {
            return;
        }
        match presentation {
            WorkspacePresentation::Agents => self
                .agent_console
                .update(cx, |view, cx| view.request_surface_focus(cx)),
            WorkspacePresentation::Terminal => self.terminal.update(cx, |terminal, cx| {
                terminal.request_focus();
                cx.notify();
            }),
            WorkspacePresentation::Main | WorkspacePresentation::Editor => {}
        }
    }

    pub fn presentation(&self) -> WorkspacePresentation {
        self.presentation
    }

    /// The presentation on screen: a focused request counts only once the
    /// account is known to hold the Dev capability.
    pub(super) fn active_presentation(&self) -> WorkspacePresentation {
        if self.presentation.is_focused() && self.can_access_mode(AppMode::Dev) {
            self.presentation
        } else {
            WorkspacePresentation::Main
        }
    }

    pub(super) fn focused_presentation_active(&self) -> bool {
        self.active_presentation().is_focused()
    }

    /// A focused window follows its Dev view: Agents, Terminal and Editor
    /// stay focused, every other destination opens in the full Dev shell,
    /// including when the saved mode was User or Support.
    pub(super) fn follow_active_view(&mut self, cx: &mut Context<Self>) {
        let current = self.active_presentation();
        let next = current.after_view_change(self.active_view);
        if next == current {
            return;
        }
        self.presentation_focus_pending = false;
        if next.is_focused() {
            self.presentation = next;
        } else {
            self.presentation = WorkspacePresentation::Main;
            self.set_mode(AppMode::Dev, cx);
        }
    }

    pub(super) fn reconcile_presentation(&mut self, cx: &mut Context<Self>) {
        let resolved = self
            .presentation
            .after_auth(self.signed_in(), self.can_access_mode(AppMode::Dev));
        if resolved != self.presentation {
            let message = if self.presentation == WorkspacePresentation::Agents {
                t!("agents.presentation.unavailable")
            } else {
                t!("presentation.unavailable")
            };
            self.presentation = resolved;
            self.presentation_focus_pending = false;
            self.show_toast(message.to_string(), ToastLevel::Warning, cx);
        }
        if self.focused_presentation_active() && self.presentation_focus_pending {
            self.presentation_focus_pending = false;
            self.select_presentation_landing(self.presentation, cx);
        }
    }

    /// What each view of the switcher wants from the user, in
    /// [`WorkspacePresentation::ALL`] order.
    pub(super) fn presentation_attention(&self, cx: &App) -> [ViewAttention; 4] {
        let on_screen = (self.effective_mode() == AppMode::Dev && !self.settings_open)
            .then_some(self.active_view);
        switcher_attention(
            on_screen,
            self.agents_attention,
            self.terminal.read(cx).attention(),
        )
    }

    /// The titlebar segmented control between the full app and the focused
    /// views. Icon-only so its width never changes with the selection. A dot
    /// marks a view that needs the user or has something new.
    pub(super) fn render_presentation_switcher(
        current: WorkspacePresentation,
        attention: [ViewAttention; 4],
        handle: &WeakEntity<Self>,
    ) -> impl IntoElement {
        let keys = crate::shortcut_reference::label(BindableAction::NextView);
        let mut group = div()
            .id("titlebar-presentation")
            .flex()
            .items_center()
            .flex_shrink_0()
            .h(px(28.0))
            .p(px(2.0))
            .gap(px(2.0))
            .rounded(px(7.0))
            .bg(ShellDeckColors::badge_bg());
        for (presentation, attention) in WorkspacePresentation::ALL.into_iter().zip(attention) {
            let active = presentation == current;
            let name = match attention {
                ViewAttention::NeedsYou => t!(
                    "presentation.attention.needs_you",
                    name = presentation.label()
                )
                .to_string(),
                ViewAttention::Finished => t!(
                    "presentation.attention.finished",
                    name = presentation.label()
                )
                .to_string(),
                ViewAttention::None => presentation.label(),
            };
            let tooltip: SharedString = match &keys {
                Some(keys) if active => t!(
                    "presentation.tooltip_active",
                    name = name,
                    keys = keys.clone()
                )
                .to_string()
                .into(),
                _ => name.into(),
            };
            let workspace = handle.clone();
            let mut segment = div()
                .id(presentation.switcher_id())
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(24.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .tooltip(move |_, cx| {
                    cx.new(|_| WorkspaceTooltip {
                        label: tooltip.clone(),
                    })
                    .into()
                })
                .on_click(move |_, _, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |this, cx| this.set_presentation(presentation, cx));
                    }
                });
            segment = if active {
                segment
                    .bg(ShellDeckColors::primary().opacity(0.16))
                    .child(lucide_icon(
                        presentation.icon(),
                        15.0,
                        ShellDeckColors::primary(),
                    ))
            } else {
                segment
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .child(lucide_icon(
                        presentation.icon(),
                        15.0,
                        ShellDeckColors::text_muted(),
                    ))
            };
            let dot = match attention {
                ViewAttention::NeedsYou => Some(ShellDeckColors::warning()),
                ViewAttention::Finished => Some(ShellDeckColors::primary()),
                ViewAttention::None => None,
            };
            if let Some(color) = dot {
                segment = segment.child(
                    div()
                        .absolute()
                        .top(px(2.0))
                        .right(px(3.0))
                        .size(px(7.0))
                        .rounded_full()
                        .bg(color)
                        .border_1()
                        .border_color(ShellDeckColors::badge_bg()),
                );
            }
            group = group.child(segment);
        }
        group
    }
}

#[cfg(test)]
mod tests {
    use super::{agent_surface_visible, ActiveView, AppMode, WorkspacePresentation};

    #[test]
    fn sdtest_2636_agents_launch_waits_for_auth_and_clamps_staff_capabilities() {
        let pending = WorkspacePresentation::Agents.after_auth(false, false);
        assert_eq!(pending, WorkspacePresentation::Agents);
        assert_eq!(pending.effective_mode(false, AppMode::User), AppMode::User);
        for allowed_mode in [AppMode::User, AppMode::Support] {
            let denied = pending.after_auth(true, false);
            assert_eq!(denied, WorkspacePresentation::Main);
            assert_eq!(denied.effective_mode(false, allowed_mode), allowed_mode);
        }
        assert_eq!(
            pending.after_auth(true, true),
            WorkspacePresentation::Agents
        );
    }

    #[test]
    fn sdtest_2637_agents_presentation_restores_saved_mode_without_persisting_dev() {
        for saved in [AppMode::User, AppMode::Support, AppMode::Dev] {
            let requested = WorkspacePresentation::Agents.after_auth(true, true);
            assert_eq!(requested.effective_mode(true, saved), AppMode::Dev);
            assert_eq!(
                WorkspacePresentation::Main.effective_mode(true, saved),
                saved
            );
            assert_eq!(
                requested.after_auth(true, false),
                WorkspacePresentation::Main
            );
        }
    }

    // SDTEST-3087 / SDUC-575
    #[test]
    fn sdtest_3087_every_focused_view_waits_for_auth_and_never_persists_dev() {
        for focused in [
            WorkspacePresentation::Agents,
            WorkspacePresentation::Terminal,
            WorkspacePresentation::Editor,
        ] {
            // Signed out: the request waits behind the welcome screen.
            assert_eq!(focused.after_auth(false, false), focused);
            // A customer or Support-only account falls back to its own app.
            assert_eq!(focused.after_auth(true, false), WorkspacePresentation::Main);
            for saved in [AppMode::User, AppMode::Support, AppMode::Dev] {
                assert_eq!(focused.effective_mode(true, saved), AppMode::Dev);
                assert_eq!(focused.effective_mode(false, saved), saved);
            }
        }
    }

    // SDTEST-3088 / SDUC-575
    #[test]
    fn sdtest_3088_a_focused_window_follows_its_view_or_opens_the_full_app() {
        use WorkspacePresentation::{Agents, Editor, Main, Terminal};
        let owned = [
            (ActiveView::Agents, Agents),
            (ActiveView::Terminal, Terminal),
            (ActiveView::FileEditor, Editor),
        ];
        for from in [Agents, Terminal, Editor] {
            // Agents, Terminal and Editor hand over to one another focused.
            for (view, owner) in owned {
                assert_eq!(from.after_view_change(view), owner, "{from:?} {view:?}");
            }
            // Every other destination needs the rail and menus around it.
            for view in [
                ActiveView::Scripts,
                ActiveView::Recent,
                ActiveView::Workspaces,
                ActiveView::MoniqueConsole,
                ActiveView::Fleet,
                ActiveView::BextCloud,
                ActiveView::Support,
            ] {
                assert_eq!(from.after_view_change(view), Main, "{from:?} {view:?}");
            }
        }
        // The full app never becomes focused by navigating inside it.
        for (view, _) in owned {
            assert_eq!(Main.after_view_change(view), Main, "{view:?}");
        }
    }

    // SDTEST-3099 / SDUC-576. The switcher points at the view that needs the
    // user, never at the one they are already looking at.
    #[test]
    fn sdtest_3099_the_switcher_marks_only_views_that_are_not_on_screen() {
        use super::{switcher_attention, ViewAttention};
        let (needs, finished, none) = (
            ViewAttention::NeedsYou,
            ViewAttention::Finished,
            ViewAttention::None,
        );
        // Order: full app, Agents, Terminal, Editor.
        assert_eq!(
            switcher_attention(Some(ActiveView::FileEditor), finished, needs),
            [none, finished, needs, none]
        );
        assert_eq!(
            switcher_attention(Some(ActiveView::Terminal), finished, needs),
            [none, finished, none, none]
        );
        assert_eq!(
            switcher_attention(Some(ActiveView::Agents), finished, needs),
            [none, none, needs, none]
        );
        // User or Support surface, or Settings open: no Dev view is on screen.
        assert_eq!(
            switcher_attention(None, needs, finished),
            [none, needs, finished, none]
        );
    }

    // SDTEST-3089 / SDUC-575
    #[test]
    fn sdtest_3089_the_cycle_shortcut_visits_every_view_once() {
        let mut seen = vec![WorkspacePresentation::Main];
        let mut current = WorkspacePresentation::Main.next();
        while current != WorkspacePresentation::Main {
            assert!(!seen.contains(&current), "{current:?} visited twice");
            seen.push(current);
            current = current.next();
        }
        assert_eq!(seen, WorkspacePresentation::ALL);
        // One switcher segment and one window title per view.
        for (index, presentation) in WorkspacePresentation::ALL.into_iter().enumerate() {
            for other in &WorkspacePresentation::ALL[index + 1..] {
                assert_ne!(presentation.switcher_id(), other.switcher_id());
                assert_ne!(presentation.window_title(), other.window_title());
                assert_ne!(presentation.icon(), other.icon());
            }
        }
    }

    #[test]
    fn sdtest_2638_agents_poll_visibility_follows_auth_mode_settings_and_selection() {
        assert!(agent_surface_visible(true, AppMode::Dev, false, true));
        assert!(!agent_surface_visible(false, AppMode::Dev, false, true));
        assert!(!agent_surface_visible(true, AppMode::User, false, true));
        assert!(!agent_surface_visible(true, AppMode::Support, false, true));
        assert!(!agent_surface_visible(true, AppMode::Dev, true, true));
        assert!(!agent_surface_visible(true, AppMode::Dev, false, false));
    }
}
