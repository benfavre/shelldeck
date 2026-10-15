use adabraka_ui::prelude::use_theme;
use gpui::prelude::*;
use gpui::{div, App, Context, Entity, Render, Subscription, Window};
use std::rc::Rc;

use crate::ai_assistant::AiAssistantView;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

/// Binary-owned callback that mirrors the standalone Dock's native visibility
/// into the shared Workspace state.
pub type AiDockVisibilityHandler = Rc<dyn Fn(bool, &mut App)>;

pub fn dock_window_title() -> String {
    t!("ai.dock.title").to_string()
}

pub fn dock_tray_label() -> String {
    t!("ai.dock.tray_open").to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub show: String,
    pub assistant: String,
    pub clippy: String,
    pub palette: String,
    pub choose_character: String,
    pub pause_character: String,
    pub return_character: String,
    pub quit: String,
    pub pinned: String,
    pub no_pinned: String,
}

impl TrayLabels {
    pub fn localized() -> Self {
        Self {
            show: t!("tray.show").to_string(),
            assistant: dock_tray_label(),
            clippy: t!("tray.clippy").to_string(),
            palette: t!("tray.palette").to_string(),
            choose_character: t!("tray.character.choose").to_string(),
            pause_character: t!("tray.character.pause").to_string(),
            return_character: t!("tray.character.return_to_dock").to_string(),
            quit: t!("tray.quit").to_string(),
            pinned: t!("tray.pinned").to_string(),
            no_pinned: t!("tray.no_pinned").to_string(),
        }
    }
}

impl Default for TrayLabels {
    fn default() -> Self {
        Self::localized()
    }
}

pub fn tray_counter_ssh(n: usize) -> String {
    match n {
        0 => t!("tray.counter.ssh.zero").to_string(),
        1 => t!("tray.counter.ssh.one").to_string(),
        n => t!("tray.counter.ssh.many", count = n).to_string(),
    }
}

pub fn tray_counter_tunnels(n: usize) -> String {
    match n {
        0 => t!("tray.counter.tunnels.zero").to_string(),
        1 => t!("tray.counter.tunnels.one").to_string(),
        n => t!("tray.counter.tunnels.many", count = n).to_string(),
    }
}

pub fn tray_counter_tickets(n: usize) -> String {
    match n {
        0 => t!("tray.counter.tickets.zero").to_string(),
        1 => t!("tray.counter.tickets.one").to_string(),
        n => t!("tray.counter.tickets.many", count = n).to_string(),
    }
}

pub fn tray_counter_monique(n: usize) -> String {
    match n {
        0 => t!("tray.counter.monique.zero").to_string(),
        1 => t!("tray.counter.monique.one").to_string(),
        n => t!("tray.counter.monique.many", count = n).to_string(),
    }
}

pub fn tray_counter_ai_tasks(n: usize) -> String {
    match n {
        0 => t!("tray.counter.ai_tasks.zero").to_string(),
        1 => t!("tray.counter.ai_tasks.one").to_string(),
        n => t!("tray.counter.ai_tasks.many", count = n).to_string(),
    }
}

/// The standalone assistant keeps its conversation entity while docking,
/// dragging between displays, hiding and reopening.
pub type AiDockDisplaysHandler = Rc<dyn Fn(&Window, &App) -> Vec<layout::DockDisplay>>;

pub mod layout;
use crate::window_chrome::{control_button, resize_edge};
use adabraka_ui::prelude::{
    Button, ButtonSize, ButtonVariant, IconSource, PopoverMenu, PopoverMenuItem,
};
use gpui::{
    Animation, AnimationExt, Bounds, Decorations, MouseButton, Pixels, Point, SharedString,
    WindowControlArea,
};
use layout::{
    dock_bounds, floating_bounds, native_origin, tiled_side, DockDisplay, DockMode, DockSide,
    DockVisibility,
};
use std::time::Duration;

const SLIDE_MS: u64 = 240;

pub struct AiDockView {
    assistant: Entity<AiAssistantView>,
    font_family: Option<String>,
    mode: DockMode,
    positioning_supported: bool,
    initial_anchor_allowed: bool,
    displays: Vec<DockDisplay>,
    resolve_displays: AiDockDisplaysHandler,
    menu_position: Option<Point<Pixels>>,
    activation_armed: bool,
    visibility: DockVisibility,
    on_open_change: AiDockVisibilityHandler,
    _activation_sub: Subscription,
    _bounds_sub: Subscription,
}

impl AiDockView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        assistant: Entity<AiAssistantView>,
        font_family: Option<String>,
        on_open_change: AiDockVisibilityHandler,
        mode: DockMode,
        positioning_supported: bool,
        displays: Vec<DockDisplay>,
        resolve_displays: AiDockDisplaysHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title(&dock_window_title());
        on_open_change(true, cx);
        let activation_sub = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.activation_armed = true;
            } else {
                let should_close = this.activation_armed
                    && window.is_window_visible()
                    && matches!(this.mode, DockMode::Attached(_))
                    && !this.visibility.closing;
                this.activation_armed = false;
                if should_close {
                    this.close(window, cx);
                }
            }
        });
        let bounds_sub = cx.observe_window_bounds(window, |this, window, cx| {
            this.sync_native_frame(window, cx);
        });
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = view.update(cx, |view, cx| view.close(window, cx));
            false
        });
        let mut visibility = DockVisibility::default();
        visibility.show();
        Self {
            assistant,
            font_family,
            mode,
            positioning_supported,
            initial_anchor_allowed: positioning_supported,
            displays,
            resolve_displays,
            menu_position: None,
            activation_armed: false,
            visibility,
            on_open_change,
            _activation_sub: activation_sub,
            _bounds_sub: bounds_sub,
        }
    }

    pub fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant
            .update(cx, |assistant, cx| assistant.focus_composer(window, cx));
    }

    pub fn is_closing(&self) -> bool {
        self.visibility.closing
    }

    pub fn initial_anchor_allowed(&self) -> bool {
        self.initial_anchor_allowed
    }

    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        tracing::debug!(mode = ?self.mode, "showing assistant window");
        self.displays = (self.resolve_displays)(window, cx);
        if !window.is_window_visible() || self.visibility.closing {
            self.visibility.show();
        }
        if self.positioning_supported
            && !self
                .displays
                .iter()
                .any(|display| display.work_area.contains(&window.bounds().center()))
        {
            if let Some(display) = layout::initial_display(&self.displays).cloned() {
                self.dock(display, DockSide::Right, window, cx);
            }
        }
        (self.on_open_change)(true, cx);
        window.show_window();
        window.activate_window();
        self.focus_composer(window, cx);
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.visibility.closing {
            return;
        }
        self.activation_armed = false;
        self.menu_position = None;
        (self.on_open_change)(false, cx);
        let generation = self.visibility.close();
        tracing::debug!(generation, "closing assistant window");
        if cx.prefers_reduced_motion() {
            window.hide_window();
        } else {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(SLIDE_MS))
                    .await;
                let _ = this.update_in(cx, |this, window, _| {
                    if this.visibility.can_finish_close(generation) {
                        tracing::debug!(generation, "hiding assistant after slide");
                        window.hide_window();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn current_display(&self, window: &Window, cx: &App) -> Option<DockDisplay> {
        let id = window.display(cx).map(|display| display.id());
        let matching: Vec<_> = self
            .displays
            .iter()
            .filter(|display| Some(display.id) == id)
            .collect();
        if matching.len() == 1 {
            return Some(matching[0].clone());
        }
        let center = window.bounds().center();
        self.displays
            .iter()
            .find(|display| display.work_area.contains(&center))
            .or_else(|| layout::initial_display(&self.displays))
            .cloned()
    }

    fn place(
        &mut self,
        bounds: Bounds<Pixels>,
        display: &DockDisplay,
        mode: DockMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.initial_anchor_allowed = false;
        if !self.positioning_supported {
            return;
        }
        if window.is_maximized() {
            window.zoom_window();
        }
        let origin = native_origin(bounds.origin, display.origin_scale);
        if let Err(error) = window.set_window_origin(origin) {
            tracing::warn!(%error, "could not place the assistant window");
            return;
        }
        window.resize(bounds.size);
        // Resizing can preserve the native bottom edge on macOS; reassert top.
        let _ = window.set_window_origin(origin);
        self.mode = mode;
        self.menu_position = None;
        cx.notify();
    }

    fn dock(
        &mut self,
        display: DockDisplay,
        side: DockSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.place(
            dock_bounds(display.work_area, side),
            &display,
            DockMode::Attached(side),
            window,
            cx,
        );
    }

    fn detach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.initial_anchor_allowed = false;
        if matches!(self.mode, DockMode::Attached(_)) {
            self.displays = (self.resolve_displays)(window, cx);
            if let Some(display) = self.current_display(window, cx) {
                let bounds = floating_bounds(display.work_area, window.bounds());
                self.place(bounds, &display, DockMode::Floating, window, cx);
            }
            // Native Wayland movement/resizing still works even though exact
            // placement is owned by the compositor.
            self.mode = DockMode::Floating;
            cx.notify();
        }
    }

    fn refresh_native_mode(&mut self, window: &Window, cx: &App) {
        if self.positioning_supported {
            if let Some(display) = self.current_display(window, cx) {
                self.mode = tiled_side(window.bounds(), display.work_area)
                    .map(DockMode::Attached)
                    .unwrap_or(DockMode::Floating);
            }
        } else if let Decorations::Client { tiling } = window.window_decorations() {
            self.mode = match (tiling.left, tiling.right, tiling.top, tiling.bottom) {
                (true, false, true, true) => DockMode::Attached(DockSide::Left),
                (false, true, true, true) => DockMode::Attached(DockSide::Right),
                _ => DockMode::Floating,
            };
        }
    }

    fn sync_native_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.refresh_native_mode(window, cx);
        cx.notify();
    }

    fn titlebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .h(px(36.0))
            .px(px(5.0))
            .gap(px(2.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .id("ai-window-drag")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .px(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .window_control_area(WindowControlArea::Drag)
                    .child(dock_window_title())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                            if resize_edge(event.position, gpui::px(5.0), window.viewport_size())
                                .is_some()
                            {
                                return;
                            }
                            this.detach(window, cx);
                            window.start_window_move();
                        }),
                    )
                    .on_mouse_down(MouseButton::Right, |event, window, _| {
                        window.show_window_menu(event.position)
                    }),
            );
        for (id, icon, key, side) in [
            (
                "ai-window-left",
                "chevron-left",
                "ai.dock.snap_left",
                DockSide::Left,
            ),
            (
                "ai-window-right",
                "chevron-right",
                "ai.dock.snap_right",
                DockSide::Right,
            ),
        ] {
            bar = bar.child(
                Button::new(id, "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from(icon))
                    .disabled(!self.positioning_supported)
                    .tooltip(
                        t!(if self.positioning_supported {
                            key
                        } else {
                            "ai.dock.system_placement"
                        })
                        .to_string(),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.displays = (this.resolve_displays)(window, cx);
                        if let Some(display) = this.current_display(window, cx) {
                            this.dock(display, side, window, cx);
                        }
                    })),
            );
        }
        if self.positioning_supported && matches!(self.mode, DockMode::Attached(_)) {
            bar = bar.child(
                Button::new("ai-window-detach", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("external-link"))
                    .tooltip(t!("ai.dock.detach").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.detach(window, cx))),
            );
        }
        bar = bar.child(
            Button::new("ai-window-monitors", "")
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("chevron-down"))
                .disabled(!self.positioning_supported)
                .tooltip(
                    t!(if self.positioning_supported {
                        "ai.dock.choose_display"
                    } else {
                        "ai.dock.system_placement"
                    })
                    .to_string(),
                )
                .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                    this.displays = (this.resolve_displays)(window, cx);
                    this.menu_position = this.menu_position.is_none().then_some(event.position());
                    cx.notify();
                })),
        );
        if self.mode == DockMode::Floating {
            bar = bar
                .child(
                    control_button(
                        "ai-window-minimize",
                        "images/minimize.svg",
                        WindowControlArea::Min,
                        false,
                    )
                    .on_click(|_, window, _| window.minimize_window()),
                )
                .child(
                    control_button(
                        "ai-window-maximize",
                        if window.is_maximized() {
                            "images/restore.svg"
                        } else {
                            "images/maximize.svg"
                        },
                        WindowControlArea::Max,
                        false,
                    )
                    .on_click(|_, window, _| window.zoom_window()),
                );
        }
        bar = bar.child(
            control_button(
                "ai-window-close",
                "images/close.svg",
                WindowControlArea::Close,
                true,
            )
            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
        );
        if !window.is_maximized() {
            if self.mode.rounds_left() {
                bar = bar.rounded_tl(use_theme().tokens.radius_xl);
            }
            if self.mode.rounds_right() {
                bar = bar.rounded_tr(use_theme().tokens.radius_xl);
            }
        }
        bar
    }
}

impl Render for AiDockView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A compositor can change tiling edges without changing window size.
        // Consult the live decoration flags on repaint as well as resize.
        if !self.positioning_supported {
            self.refresh_native_mode(window, cx);
        }
        let floating = self.mode == DockMode::Floating && !window.is_maximized();
        window.set_client_inset(if floating {
            gpui::px(5.0)
        } else {
            gpui::px(0.0)
        });
        let round_left = !window.is_maximized() && self.mode.rounds_left();
        let round_right = !window.is_maximized() && self.mode.rounds_right();
        self.assistant.update(cx, |assistant, cx| {
            assistant.set_dock_corners(round_left, round_right, cx)
        });
        let mut surface = div()
            .id("ai-dock-surface")
            .absolute()
            .top_0()
            .bottom_0()
            .w_full()
            .right_0()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h(px(0.0))
            .overflow_hidden()
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .child(self.titlebar(window, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h(px(0.0))
                    .child(self.assistant.clone()),
            );
        let radius = use_theme().tokens.radius_xl;
        if round_left {
            surface = surface.rounded_tl(radius).rounded_bl(radius);
        }
        if round_right {
            surface = surface.rounded_tr(radius).rounded_br(radius);
        }
        let closing = self.visibility.closing;
        let width = window.viewport_size().width;
        let direction = if self.mode == DockMode::Attached(DockSide::Left) {
            1.0
        } else {
            -1.0
        };
        let animated = surface.with_animation(
            SharedString::from(format!("ai-dock-slide-{}", self.visibility.generation())),
            Animation::new(Duration::from_millis(SLIDE_MS))
                .with_easing(|t| 1.0 - (1.0 - t).powi(3)),
            move |element, delta| {
                let hidden = if closing { delta } else { 1.0 - delta };
                element.right(width * (direction * hidden))
            },
        );
        let mut root = div()
            .id("ai-dock-root")
            .relative()
            .size_full()
            .overflow_hidden()
            .p(if floating {
                gpui::px(5.0)
            } else {
                gpui::px(0.0)
            })
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape")
                    && !this.assistant.read(cx).has_open_dialog()
                {
                    this.close(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(div().relative().size_full().child(animated))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    if !window.is_maximized() {
                        if let Some(edge) =
                            resize_edge(event.position, gpui::px(5.0), window.viewport_size())
                        {
                            this.mode = DockMode::Floating;
                            this.initial_anchor_allowed = false;
                            window.start_window_resize(edge);
                            cx.notify();
                        }
                    }
                }),
            );
        if !window.is_maximized() {
            root = root
                .child(crate::window_chrome::resize_cursor_layer())
                .on_mouse_move(|_, window, _| window.refresh());
        }
        if let Some(position) = self.menu_position {
            let mut items = Vec::new();
            for (number, display) in self.displays.iter().enumerate() {
                let label = if display.primary {
                    t!("ai.dock.main_display").to_string()
                } else if !display.label.is_empty() {
                    display.label.clone()
                } else {
                    t!("ai.dock.display_number", number = number + 1).to_string()
                };
                for (key, side) in [
                    ("ai.dock.snap_left", DockSide::Left),
                    ("ai.dock.snap_right", DockSide::Right),
                ] {
                    let display = display.clone();
                    let weak = cx.weak_entity();
                    items.push(
                        PopoverMenuItem::new(
                            format!("ai-dock-monitor-{number}-{side:?}"),
                            format!("{label} · {}", t!(key)),
                        )
                        .on_click(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.dock(display.clone(), side, window, cx)
                            });
                        }),
                    );
                }
            }
            let weak = cx.weak_entity();
            root = root.child(PopoverMenu::new(position, items).on_close(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.menu_position = None;
                    cx.notify();
                });
            }));
        }
        if let Some(font_family) = &self.font_family {
            root = root.font_family(font_family.clone());
        }
        root
    }
}
