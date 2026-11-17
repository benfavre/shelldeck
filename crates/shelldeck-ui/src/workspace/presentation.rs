//! Transient workspace presentation: the full app or one focused Dev view.
//! Authentication and the persisted app mode remain authoritative.
use super::*;
use crate::shortcut_reference::BindableAction;
use crate::view_attention::ViewAttention;
use adabraka_ui::overlays::popover::PopoverContent;
use adabraka_ui::prelude::Popover;

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
    /// the full app, the session navigator of Agents or the session list of
    /// Terminal. The focused Editor view has none.
    pub(super) fn has_sidebar_toggle(self) -> bool {
        !matches!(self, Self::Editor)
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
        self.pro_open = false;
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
        self.pro_open = false;
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
        let was_pro_open = std::mem::replace(&mut self.pro_open, false);
        if presentation.is_focused() {
            self.request_presentation(presentation, cx);
            return;
        }
        if !self.presentation.is_focused() {
            if was_pro_open {
                self.on_active_view_changed(cx);
                cx.notify();
            }
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

    /// One row of a title-bar dropdown: icon, label, optional dot.
    fn titlebar_menu_row(
        id: &'static str,
        icon: &'static str,
        label: String,
        active: bool,
        dot: Option<Hsla>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(34.0))
            .px(px(10.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .when(active, |row| {
                row.bg(ShellDeckColors::primary().opacity(0.10))
                    .text_color(ShellDeckColors::primary())
            })
            .when(!active, |row| {
                row.text_color(ShellDeckColors::text_primary())
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            })
            .child(lucide_icon(
                icon,
                15.0,
                if active {
                    ShellDeckColors::primary()
                } else {
                    ShellDeckColors::text_muted()
                },
            ))
            .child(div().flex_1().child(label))
            .when_some(dot, |row, color| {
                row.child(div().size(px(7.0)).rounded_full().bg(color))
            })
    }

    /// The view chooser of every title bar (SDUC-596): the current view as a
    /// « ▾ » chip, the four views in its dropdown with their attention dots.
    pub(super) fn render_presentation_menu(
        current: WorkspacePresentation,
        attention: [ViewAttention; 4],
        compact: bool,
        handle: &WeakEntity<Self>,
    ) -> AnyElement {
        let handle = handle.clone();
        let tooltip: SharedString =
            match crate::shortcut_reference::label(BindableAction::NextView) {
                Some(keys) => t!(
                    "presentation.tooltip_active",
                    name = current.label(),
                    keys = keys
                )
                .to_string(),
                None => current.label(),
            }
            .into();
        // At rest the chip keeps the old switcher's signal: a dot when another
        // view needs the user (warning) or has something new (primary).
        let others = WorkspacePresentation::ALL
            .into_iter()
            .zip(attention)
            .filter(|(presentation, _)| *presentation != current)
            .map(|(_, attention)| attention)
            .collect::<Vec<_>>();
        let rest_dot = if others.contains(&ViewAttention::NeedsYou) {
            Some(ShellDeckColors::warning())
        } else if others.contains(&ViewAttention::Finished) {
            Some(ShellDeckColors::primary())
        } else {
            None
        };
        Popover::new("titlebar-view-menu")
            .anchor(Corner::TopLeft)
            .trigger(
                div()
                    .id("titlebar-view-chip")
                    .debug_selector(|| "titlebar-view-chip".into())
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(34.0))
                    .px(px(12.0))
                    .rounded(px(9.0))
                    .bg(ShellDeckColors::badge_bg().opacity(0.7))
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::badge_bg()))
                    .text_size(px(13.5))
                    .text_color(ShellDeckColors::text_primary())
                    .tooltip(move |_, cx| {
                        cx.new(|_| WorkspaceTooltip {
                            label: tooltip.clone(),
                        })
                        .into()
                    })
                    .child(lucide_icon(
                        current.icon(),
                        15.0,
                        ShellDeckColors::primary(),
                    ))
                    // A narrow title bar keeps the icon; the view is in the tooltip.
                    .when(!compact, |chip| chip.child(current.label()))
                    .when_some(rest_dot, |chip, color| {
                        chip.child(
                            div()
                                .debug_selector(|| "titlebar-view-attention".into())
                                .size(px(7.0))
                                .rounded_full()
                                .bg(color),
                        )
                    })
                    .child(lucide_icon(
                        "chevron-down",
                        14.0,
                        ShellDeckColors::text_muted(),
                    )),
            )
            .content(move |window, cx| {
                let handle = handle.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_, cx| {
                        let mut menu = div().flex().flex_col().gap(px(2.0)).min_w(px(200.0));
                        for (presentation, attention) in
                            WorkspacePresentation::ALL.into_iter().zip(attention)
                        {
                            let dot = match attention {
                                ViewAttention::NeedsYou => Some(ShellDeckColors::warning()),
                                ViewAttention::Finished => Some(ShellDeckColors::primary()),
                                ViewAttention::None => None,
                            };
                            let workspace = handle.clone();
                            menu = menu.child(
                                Self::titlebar_menu_row(
                                    presentation.switcher_id(),
                                    presentation.icon(),
                                    presentation.label(),
                                    presentation == current,
                                    dot,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        if let Some(workspace) = workspace.upgrade() {
                                            workspace.update(cx, |this, cx| {
                                                this.set_presentation(presentation, cx)
                                            });
                                        }
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            );
                        }
                        menu.into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    /// « Inklura ▾ »: the product menu of a focused view, which has no menu row.
    pub(super) fn render_brand_menu(compact: bool, handle: &WeakEntity<Self>) -> AnyElement {
        let handle = handle.clone();
        let mut face = div()
            .id("titlebar-brand")
            .debug_selector(|| "titlebar-brand".into())
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.0))
            .h(px(36.0))
            .pl(px(4.0))
            .pr(px(6.0))
            .rounded(px(9.0))
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(crate::brand::brand_badge(28.0));
        if !compact {
            face = face.child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .whitespace_nowrap()
                    .child("Inklura"),
            );
        }
        face = face.child(lucide_icon(
            "chevron-down",
            14.0,
            ShellDeckColors::text_muted(),
        ));
        Popover::new("titlebar-brand-menu")
            .anchor(Corner::TopLeft)
            .trigger(face)
            .content(move |window, cx| {
                let handle = handle.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_, cx| {
                        let full = handle.clone();
                        let settings = handle.clone();
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .min_w(px(220.0))
                            .child(
                                Self::titlebar_menu_row(
                                    "titlebar-brand-full",
                                    WorkspacePresentation::Main.icon(),
                                    WorkspacePresentation::Main.label(),
                                    false,
                                    None,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        if let Some(workspace) = full.upgrade() {
                                            workspace.update(cx, |this, cx| {
                                                this.set_presentation(
                                                    WorkspacePresentation::Main,
                                                    cx,
                                                )
                                            });
                                        }
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            )
                            .child(
                                Self::titlebar_menu_row(
                                    "titlebar-brand-settings",
                                    "settings",
                                    t!("agents.cockpit.settings").to_string(),
                                    false,
                                    None,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        if let Some(workspace) = settings.upgrade() {
                                            workspace.update(cx, |this, cx| this.open_settings(cx));
                                        }
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            )
                            .child(
                                Self::titlebar_menu_row(
                                    "titlebar-brand-help",
                                    "circle-help",
                                    t!("agents.cockpit.help").to_string(),
                                    false,
                                    None,
                                )
                                .on_click(cx.listener(
                                    |_, _, _, cx| {
                                        let _ =
                                            shelldeck_core::config::cloud_account::open_in_browser(
                                                "https://shelldeck.1clic.pro",
                                            );
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            )
                            .into_any_element()
                    })
                })
            })
            .into_any_element()
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
