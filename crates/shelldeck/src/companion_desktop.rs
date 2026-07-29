use std::time::{Duration, Instant};

use gpui::{
    div, img, prelude::*, px, AnyWindowHandle, App, AppContext, Bounds, Context, Entity,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels,
    Point, Render, Size, Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind,
    WindowOptions,
};
use shelldeck_core::ai::{
    ClippyConfig, CompanionCharacterId, CompanionMotionPreference, CompanionScale,
    DesktopCompanionMovement,
};
use shelldeck_core::companion::geometry::{Point2, Rect};
use shelldeck_core::companion::simulation::{
    AnimationFramePolicy, CharacterSimulation, CharacterSimulationState,
};

const STEP: Duration = Duration::from_millis(33);
const STATIC_MARGIN: f32 = 24.0;
const DRAG_THRESHOLD: f32 = 4.0;
const CLICK_NUDGE: f32 = 112.0;
const REACTION_DURATION: Duration = Duration::from_millis(650);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionRuntimeCommand {
    Pause,
    ReturnToCorner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayCapabilityTier {
    FullRoaming,
    ScreenEdgeOnly,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayDiagnostics {
    pub tier: OverlayCapabilityTier,
    pub overlay_requested: bool,
    pub overlay_open: bool,
    pub movement_supported: bool,
    pub mouse_passthrough: bool,
    pub paused: bool,
    pub reason: Option<String>,
    pub display_count: usize,
    pub geometry_snapshot_count: u64,
    pub simulation_steps: u64,
    pub native_moves: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesktopDisplay {
    pub id: Option<gpui::DisplayId>,
    pub bounds: Bounds<Pixels>,
    pub work_area: Bounds<Pixels>,
    pub scale_factor: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeRoute {
    pub enabled: bool,
    pub requested_desktop: bool,
    pub unavailable: bool,
}

pub fn runtime_route(config: &ClippyConfig) -> RuntimeRoute {
    let requested_desktop = config.appearance.desktop.enabled
        && config.appearance.character_id() != CompanionCharacterId::None;
    RuntimeRoute {
        enabled: requested_desktop,
        requested_desktop,
        unavailable: !requested_desktop,
    }
}

fn frame_elapsed_millis(last_tick: Option<Instant>, now: Instant) -> u64 {
    last_tick
        .map(|last| {
            now.saturating_duration_since(last)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        })
        .unwrap_or(STEP.as_millis() as u64)
}

#[derive(Debug, Clone)]
struct RuntimeSimulation {
    simulation: CharacterSimulation,
    display: DesktopDisplay,
    logical_extent: f32,
    extent: f32,
    pending_ms: u64,
    steps: u64,
}

impl RuntimeSimulation {
    fn new(display: DesktopDisplay, logical_extent: f32) -> Self {
        let extent = desktop_coordinate_extent(logical_extent, display.scale_factor);
        let position = safe_corner(display.work_area, extent);
        Self {
            simulation: CharacterSimulation::new(display_label(display.id), to_point2(position)),
            display,
            logical_extent,
            extent,
            pending_ms: 0,
            steps: 0,
        }
    }

    fn update_display(&mut self, display: DesktopDisplay) {
        self.display = display;
        self.extent = desktop_coordinate_extent(self.logical_extent, display.scale_factor);
        self.simulation.display_id = display_label(display.id);
        self.simulation.position = self.bounds().clamp_point(self.simulation.position);
    }

    fn return_to_corner(&mut self) {
        self.pending_ms = 0;
        self.simulation.state = CharacterSimulationState::ReturningToDock;
        self.simulation
            .set_target(to_point2(safe_corner(self.display.work_area, self.extent)));
    }

    fn place_from_drag(&mut self, origin: Point<Pixels>, extent: f32) -> Point<Pixels> {
        let origin = clamp_overlay_origin(origin, self.display.work_area, extent);
        self.simulation.position = to_point2(origin);
        self.simulation.target = None;
        self.simulation.state = CharacterSimulationState::Summoned;
        self.pending_ms = 0;
        origin
    }

    fn react_to_click(&mut self, click_count: usize, extent: f32) -> bool {
        // A second mouse-up may arrive while the first click's hop is still
        // active. Let the double-click upgrade that target into the larger
        // dash instead of dropping the user's explicit interaction.
        if self.simulation.target.is_some() && click_count < 2 {
            return false;
        }
        let target = click_reaction_target(
            self.simulation.position,
            self.display.work_area,
            extent,
            click_count >= 2,
        );
        self.simulation.set_target(target);
        self.simulation.state = CharacterSimulationState::Jumping;
        self.pending_ms = 0;
        self.simulation.remember_action(if click_count >= 2 {
            "double-click-zoom"
        } else {
            "click-hop"
        });
        true
    }

    fn pause(&mut self) {
        self.pending_ms = 0;
        self.simulation.target = None;
        self.simulation.state = CharacterSimulationState::Resting;
    }

    fn playful_target(&mut self) {
        if !self.simulation.can_start_action() {
            return;
        }
        let bounds = self.display.work_area;
        let left = f32::from(bounds.origin.x) + STATIC_MARGIN;
        let right = f32::from(bounds.right()) - self.extent - STATIC_MARGIN;
        let floor = f32::from(bounds.bottom()) - self.extent - STATIC_MARGIN;
        let next_x = if self.simulation.position.x < (left + right) * 0.5 {
            right
        } else {
            left
        };
        self.simulation
            .set_target(Point2::new(next_x, floor.max(f32::from(bounds.origin.y))));
        self.simulation.remember_action("screen-edge-hop");
    }

    fn climb_window(&mut self, window: Bounds<Pixels>) -> bool {
        if !self.simulation.can_start_action() {
            return false;
        }
        let Some(target) = window_top_target(window, self.display.work_area, self.extent) else {
            return false;
        };
        self.simulation.set_target(target);
        self.simulation.remember_action("window-climb");
        true
    }

    fn step_capped(&mut self, elapsed_ms: u64) -> bool {
        let before = self.simulation.position;
        let step_ms = self.simulation.config.fixed_timestep_ms.max(1);
        let max_steps = u64::from(self.simulation.config.max_catch_up_steps);
        let max_budget_ms = step_ms.saturating_mul(max_steps);
        self.pending_ms = self.pending_ms.saturating_add(elapsed_ms);
        let budget_ms = self.pending_ms.min(max_budget_ms);
        let steps = self.simulation.step_capped(budget_ms, self.bounds());
        let consumed_ms = u64::from(steps).saturating_mul(step_ms);
        self.pending_ms = if self.pending_ms > max_budget_ms {
            self.pending_ms % step_ms
        } else {
            self.pending_ms.saturating_sub(consumed_ms)
        };
        self.steps += u64::from(steps);
        self.simulation.position != before
    }

    fn position(&self) -> Point<Pixels> {
        from_point2(self.simulation.position)
    }

    fn moving(&self, reduced_motion: bool) -> bool {
        self.simulation.request_animation_frames(reduced_motion) == AnimationFramePolicy::Continuous
    }

    fn bounds(&self) -> Rect {
        to_rect(self.display.work_area, self.extent)
    }
}

pub struct DesktopCharacterRuntime {
    config: ClippyConfig,
    overlay: Option<WindowHandle<CharacterOverlayView>>,
    diagnostics: OverlayDiagnostics,
    simulation: Option<RuntimeSimulation>,
    last_tick: Option<Instant>,
    roam_timer_scheduled: bool,
    roam_generation: u64,
}

impl DesktopCharacterRuntime {
    pub fn new(config: ClippyConfig) -> Self {
        Self {
            diagnostics: OverlayDiagnostics {
                tier: OverlayCapabilityTier::Unavailable,
                overlay_requested: runtime_route(&config).enabled,
                overlay_open: false,
                movement_supported: false,
                mouse_passthrough: false,
                paused: false,
                reason: None,
                display_count: 0,
                geometry_snapshot_count: 0,
                simulation_steps: 0,
                native_moves: 0,
            },
            config,
            overlay: None,
            simulation: None,
            last_tick: None,
            roam_timer_scheduled: false,
            roam_generation: 0,
        }
    }

    pub fn apply_config(
        &mut self,
        runtime_entity: Entity<Self>,
        config: ClippyConfig,
        main_window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let recreate = self.config.appearance.character_id() != config.appearance.character_id()
            || self.config.appearance.scale != config.appearance.scale;
        self.config = config;
        self.roam_generation = self.roam_generation.wrapping_add(1);
        self.roam_timer_scheduled = false;
        if recreate {
            self.close_overlay(cx);
        }
        self.diagnostics = detect_capabilities(cx, runtime_route(&self.config));
        if self.diagnostics.tier == OverlayCapabilityTier::ScreenEdgeOnly
            && self.config.appearance.desktop.allow_window_climbing
        {
            self.diagnostics.tier = OverlayCapabilityTier::FullRoaming;
            self.diagnostics.reason =
                Some("External-window climbing and multi-display roaming are available".into());
        }
        if self.diagnostics.overlay_requested
            && self.diagnostics.tier != OverlayCapabilityTier::Unavailable
        {
            self.ensure_overlay(runtime_entity.clone(), main_window, cx);
            self.schedule_roam(runtime_entity, cx);
        } else {
            self.close_overlay(cx);
        }
    }

    pub fn handle_command(
        &mut self,
        runtime_entity: Entity<Self>,
        command: CompanionRuntimeCommand,
        main_window: AnyWindowHandle,
        cx: &mut App,
    ) {
        match command {
            CompanionRuntimeCommand::Pause => {
                self.diagnostics.paused = !self.diagnostics.paused;
                if let Some(sim) = &mut self.simulation {
                    if self.diagnostics.paused {
                        sim.pause();
                    } else {
                        sim.playful_target();
                    }
                }
                if !self.diagnostics.paused {
                    self.ensure_overlay(runtime_entity.clone(), main_window, cx);
                    self.request_frame(cx);
                    self.schedule_roam(runtime_entity, cx);
                }
            }
            CompanionRuntimeCommand::ReturnToCorner => {
                self.diagnostics.paused = false;
                self.last_tick = None;
                self.roam_generation = self.roam_generation.wrapping_add(1);
                self.roam_timer_scheduled = false;
                self.ensure_overlay(runtime_entity.clone(), main_window, cx);
                if let Some(sim) = &mut self.simulation {
                    sim.return_to_corner();
                }
                self.request_frame(cx);
                self.schedule_roam(runtime_entity, cx);
            }
        }
    }

    pub fn diagnostics(&self) -> &OverlayDiagnostics {
        &self.diagnostics
    }

    fn current_position(&self) -> Option<Point<Pixels>> {
        self.simulation.as_ref().map(RuntimeSimulation::position)
    }

    fn begin_user_drag(&mut self) {
        self.roam_generation = self.roam_generation.wrapping_add(1);
        self.roam_timer_scheduled = false;
        self.last_tick = None;
        if let Some(sim) = &mut self.simulation {
            sim.pause();
            sim.simulation.state = CharacterSimulationState::Summoned;
        }
    }

    fn drag_to(&mut self, origin: Point<Pixels>, cx: &App) -> Option<Point<Pixels>> {
        let displays = desktop_displays(cx);
        let extent = self.simulation.as_ref()?.extent;
        let display = display_for_overlay_origin(&displays, origin, extent)?;
        let sim = self.simulation.as_mut()?;
        sim.update_display(display);
        let origin = sim.place_from_drag(origin, sim.extent);
        self.diagnostics.display_count = displays.len();
        self.diagnostics.native_moves = self.diagnostics.native_moves.saturating_add(1);
        Some(origin)
    }

    fn finish_user_drag(&mut self, runtime_entity: Entity<Self>, cx: &mut App) {
        if let Some(sim) = &mut self.simulation {
            sim.simulation.target = None;
            sim.simulation.state = CharacterSimulationState::Resting;
            sim.simulation.remember_action("user-drag");
        }
        self.last_tick = None;
        self.schedule_roam(runtime_entity, cx);
    }

    fn react_to_click(
        &mut self,
        click_count: usize,
        runtime_entity: Entity<Self>,
        cx: &mut App,
    ) -> bool {
        if self.diagnostics.paused {
            return false;
        }
        self.roam_generation = self.roam_generation.wrapping_add(1);
        self.roam_timer_scheduled = false;
        self.last_tick = None;
        let started = self.simulation.as_mut().is_some_and(|sim| {
            let extent = sim.extent;
            sim.react_to_click(click_count, extent)
        });
        self.schedule_roam(runtime_entity, cx);
        started
    }

    fn roaming_delay(&self) -> Option<Duration> {
        if self.overlay.is_none()
            || self.diagnostics.paused
            || matches!(
                self.config.appearance.motion,
                CompanionMotionPreference::Reduced | CompanionMotionPreference::Off
            )
        {
            return None;
        }
        match self.config.appearance.desktop.movement {
            DesktopCompanionMovement::Still => None,
            DesktopCompanionMovement::Occasional => Some(Duration::from_secs(90)),
            DesktopCompanionMovement::Playful => Some(Duration::from_secs(30)),
        }
    }

    fn schedule_roam(&mut self, runtime_entity: Entity<Self>, cx: &mut App) {
        let Some(delay) = self.roaming_delay() else {
            return;
        };
        if self.roam_timer_scheduled {
            return;
        }
        self.roam_timer_scheduled = true;
        let generation = self.roam_generation;
        cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            let _ = cx.update(|cx| {
                runtime_entity.update(cx, |runtime, cx| {
                    if runtime.roam_generation != generation {
                        return;
                    }
                    runtime.roam_timer_scheduled = false;
                    if runtime.roaming_delay().is_none() {
                        return;
                    }
                    if let Some(sim) = &mut runtime.simulation {
                        sim.simulation.advance_idle_time(delay.as_millis() as u64);
                    }
                    if !runtime.target_external_window(cx) {
                        runtime.advance_display(cx);
                        if let Some(sim) = &mut runtime.simulation {
                            sim.playful_target();
                        }
                    }
                    runtime.request_frame(cx);
                    runtime.schedule_roam(runtime_entity.clone(), cx);
                });
            });
        })
        .detach();
    }

    fn advance_display(&mut self, cx: &App) {
        if !self.config.appearance.desktop.allow_multi_monitor {
            return;
        }
        let displays = desktop_displays(cx);
        if displays.len() < 2 {
            return;
        }
        let Some(sim) = &mut self.simulation else {
            return;
        };
        let current = displays
            .iter()
            .position(|display| display.id == sim.display.id)
            .unwrap_or(0);
        let next = displays[(current + 1) % displays.len()];
        if next.id == sim.display.id {
            return;
        }
        sim.update_display(next);
        sim.simulation.position = to_point2(gpui::point(
            px(f32::from(next.work_area.origin.x) + STATIC_MARGIN),
            px(
                (f32::from(next.work_area.bottom()) - sim.extent - STATIC_MARGIN)
                    .max(f32::from(next.work_area.origin.y)),
            ),
        ));
    }

    fn target_external_window(&mut self, cx: &App) -> bool {
        if !self.config.appearance.desktop.allow_window_climbing {
            return false;
        }
        let windows = cx.visible_external_window_bounds();
        self.diagnostics.geometry_snapshot_count =
            self.diagnostics.geometry_snapshot_count.saturating_add(1);
        let Some(sim) = &mut self.simulation else {
            return false;
        };
        let position = sim.simulation.position;
        let Some(window) = windows.into_iter().min_by(|a, b| {
            let distance = |bounds: &Bounds<Pixels>| {
                let center = bounds.center();
                let dx = f32::from(center.x) - position.x;
                let dy = f32::from(bounds.origin.y) - position.y;
                dx * dx + dy * dy
            };
            distance(a).total_cmp(&distance(b))
        }) else {
            return false;
        };

        let displays = desktop_displays(cx);
        if let Some(display) = displays
            .into_iter()
            .find(|display| display.work_area.contains(&window.center()))
        {
            sim.update_display(display);
        }
        sim.climb_window(window)
    }

    fn ensure_overlay(
        &mut self,
        runtime_entity: Entity<Self>,
        main_window: AnyWindowHandle,
        cx: &mut App,
    ) {
        if self.overlay.is_some() || self.diagnostics.tier == OverlayCapabilityTier::Unavailable {
            return;
        }
        let Some(display) = primary_desktop_display(cx) else {
            self.diagnostics.reason =
                Some("No display is available for the desktop character".into());
            return;
        };
        let window_size = character_window_size(self.config.appearance.scale);
        self.simulation = Some(RuntimeSimulation::new(display, window_size));
        if let Some(sim) = &mut self.simulation {
            if !matches!(
                self.config.appearance.motion,
                CompanionMotionPreference::Off | CompanionMotionPreference::Reduced
            ) && self.config.appearance.desktop.movement != DesktopCompanionMovement::Still
            {
                sim.playful_target();
            }
        }
        let logical_work_area = logical_display_bounds(cx, display.id).unwrap_or(display.work_area);
        let bounds = Bounds {
            origin: safe_corner(logical_work_area, window_size),
            size: Size {
                width: px(window_size),
                height: px(window_size),
            },
        };
        let options = WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            kind: WindowKind::Overlay,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: display.id,
            window_decorations: Some(WindowDecorations::Client),
            mouse_passthrough: false,
            focus: false,
            show: true,
            app_id: Some("shelldeck-character".to_string()),
            ..Default::default()
        };
        let character = self.config.appearance.character_id();
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| CharacterOverlayView::new(main_window, character, runtime_entity))
        }) {
            Ok(handle) => {
                self.overlay = Some(handle);
                self.diagnostics.overlay_open = true;
                self.diagnostics.mouse_passthrough = false;
                self.diagnostics.display_count = cx.displays().len();
                // `open_window` renders the new view synchronously while this
                // runtime entity is still being updated. Arm runtime-backed
                // frames on the next application turn to avoid re-entering the
                // same entity from the overlay's initial render.
                cx.spawn(async move |cx| {
                    let _ = cx.update(|cx| {
                        let _ = handle.update(cx, |view, window, cx| {
                            view.schedule_frame(window, cx);
                        });
                    });
                })
                .detach();
            }
            Err(error) => {
                self.diagnostics.tier = OverlayCapabilityTier::Unavailable;
                self.diagnostics.reason = Some(format!("Overlay creation failed: {error:#}"));
                tracing::warn!(error = %error, "desktop character overlay unavailable");
            }
        }
    }

    fn close_overlay(&mut self, cx: &mut App) {
        if let Some(handle) = self.overlay.take() {
            let _ = handle.update(cx, |_view, window, _cx| window.remove_window());
        }
        self.simulation = None;
        self.last_tick = None;
        self.roam_timer_scheduled = false;
        self.roam_generation = self.roam_generation.wrapping_add(1);
        self.diagnostics.overlay_open = false;
    }

    fn request_frame(&self, cx: &mut App) {
        if let Some(handle) = self.overlay {
            let _ = handle.update(cx, |view, window, cx| view.schedule_frame(window, cx));
        }
    }

    fn record_movement_error(&mut self, error: String) {
        self.diagnostics.reason = Some(format!("Native movement failed: {error}"));
        self.diagnostics.paused = true;
        if let Some(sim) = &mut self.simulation {
            sim.pause();
        }
    }

    fn on_frame(&mut self, cx: &mut App) -> CharacterFrameState {
        let reduced_motion = matches!(
            self.config.appearance.motion,
            CompanionMotionPreference::Reduced | CompanionMotionPreference::Off
        ) || self.config.appearance.desktop.movement
            == DesktopCompanionMovement::Still;
        if self.diagnostics.paused || reduced_motion {
            if let Some(sim) = &mut self.simulation {
                sim.pause();
            }
            return self.frame_state();
        }
        let displays = desktop_displays(cx);
        if let Some(sim) = &mut self.simulation {
            let current_display = displays
                .iter()
                .copied()
                .find(|display| display.id == sim.display.id)
                .or_else(|| displays.first().copied());
            if let Some(display) = current_display {
                sim.update_display(display);
                self.diagnostics.display_count = displays.len();
            }
            let now = Instant::now();
            let elapsed_ms = frame_elapsed_millis(self.last_tick, now);
            self.last_tick = Some(now);
            let moved = sim.step_capped(elapsed_ms);
            self.diagnostics.simulation_steps = sim.steps;
            if moved {
                self.diagnostics.native_moves += 1;
            }
        }
        self.frame_state()
    }

    fn frame_state(&self) -> CharacterFrameState {
        let reduced_motion = matches!(
            self.config.appearance.motion,
            CompanionMotionPreference::Reduced | CompanionMotionPreference::Off
        ) || self.config.appearance.desktop.movement
            == DesktopCompanionMovement::Still;
        CharacterFrameState {
            position: self.simulation.as_ref().map(|sim| sim.position()),
            moving: self
                .simulation
                .as_ref()
                .is_some_and(|sim| sim.moving(reduced_motion)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CharacterFrameState {
    position: Option<Point<Pixels>>,
    moving: bool,
}

pub struct CharacterOverlayView {
    runtime: Entity<DesktopCharacterRuntime>,
    character: CompanionCharacterId,
    main_window: AnyWindowHandle,
    frame_scheduled: bool,
    drag: Option<CharacterDrag>,
    visual_state: CharacterVisualState,
    reaction_generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct CharacterDrag {
    grab_offset: Point<Pixels>,
    start_origin: Point<Pixels>,
    moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharacterVisualState {
    Idle,
    Dragging,
    Reacting,
}

impl CharacterOverlayView {
    fn new(
        main_window: AnyWindowHandle,
        character: CompanionCharacterId,
        runtime: Entity<DesktopCharacterRuntime>,
    ) -> Self {
        Self {
            runtime,
            character,
            main_window,
            frame_scheduled: false,
            drag: None,
            visual_state: CharacterVisualState::Idle,
            reaction_generation: 0,
        }
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(CharacterDrag {
            grab_offset: event.position,
            start_origin: self
                .runtime
                .read(cx)
                .current_position()
                .unwrap_or(window.bounds().origin),
            moved: false,
        });
        self.visual_state = CharacterVisualState::Dragging;
        self.reaction_generation = self.reaction_generation.wrapping_add(1);
        self.runtime
            .update(cx, |runtime, _cx| runtime.begin_user_drag());
        cx.notify();
        cx.stop_propagation();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        let Some(mut drag) = self.drag else {
            return;
        };
        let window_origin = self
            .runtime
            .read(cx)
            .current_position()
            .unwrap_or(window.bounds().origin);
        let origin = drag_origin(
            window_origin,
            event.position,
            drag.grab_offset,
            desktop_pointer_scale(window.scale_factor()),
        );
        let dx = f32::from(origin.x) - f32::from(drag.start_origin.x);
        let dy = f32::from(origin.y) - f32::from(drag.start_origin.y);
        drag.moved |= dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD;
        self.drag = Some(drag);
        let position = self
            .runtime
            .update(cx, |runtime, cx| runtime.drag_to(origin, cx));
        if let Some(position) = position {
            if let Err(error) = window.set_window_origin(position) {
                tracing::warn!(error = %error, "desktop character drag movement failed");
                self.runtime.update(cx, |runtime, _cx| {
                    runtime.record_movement_error(error.to_string())
                });
            }
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let runtime_entity = self.runtime.clone();
        let started_motion = if drag.moved {
            self.runtime.update(cx, |runtime, cx| {
                runtime.finish_user_drag(runtime_entity.clone(), cx)
            });
            false
        } else {
            self.runtime.update(cx, |runtime, cx| {
                runtime.react_to_click(event.click_count, runtime_entity.clone(), cx)
            })
        };
        self.show_reaction(cx);
        if started_motion {
            self.schedule_frame(window, cx);
        }
        cx.stop_propagation();
    }

    fn show_reaction(&mut self, cx: &mut Context<Self>) {
        self.visual_state = CharacterVisualState::Reacting;
        self.reaction_generation = self.reaction_generation.wrapping_add(1);
        let generation = self.reaction_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REACTION_DURATION).await;
            let _ = this.update(cx, |view, cx| {
                if view.reaction_generation == generation && view.drag.is_none() {
                    view.visual_state = CharacterVisualState::Idle;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_scheduled {
            return;
        }
        self.frame_scheduled = true;
        cx.on_next_frame(window, |view, window, cx| {
            view.frame_scheduled = false;
            let state = view.runtime.update(cx, |runtime, cx| runtime.on_frame(cx));
            if let Some(position) = state.position {
                if let Err(error) = window.set_window_origin(position) {
                    tracing::warn!(error = %error, "desktop character native movement failed");
                    view.runtime.update(cx, |runtime, _cx| {
                        runtime.record_movement_error(error.to_string())
                    });
                }
            }
            if state.moving {
                view.schedule_frame(window, cx);
            }
            cx.notify();
        });
    }
}

impl Render for CharacterOverlayView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.main_window;
        let character = div()
            .id("desktop-character-interaction-surface")
            .size_full()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    view.handle_mouse_down(event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
                view.handle_mouse_move(event, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, window, cx| {
                    view.handle_mouse_up(event, window, cx)
                }),
            )
            .child(
                img(character_visual_asset(self.character, self.visual_state))
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Contain),
            );

        div()
            .size_full()
            .bg(gpui::transparent_black())
            .child(character)
    }
}

fn detect_capabilities(cx: &App, route: RuntimeRoute) -> OverlayDiagnostics {
    let mut tier = OverlayCapabilityTier::Unavailable;
    let mut reason = None;
    let mut movement_supported = false;
    if route.enabled {
        if is_wayland_session() {
            reason = Some("Wayland does not expose reliable top-level positioning or external-window geometry, so the interactive desktop character is unavailable".into());
        } else if is_x11_session() || cfg!(target_os = "windows") || cfg!(target_os = "macos") {
            tier = OverlayCapabilityTier::ScreenEdgeOnly;
            movement_supported = true;
            reason = Some(
                "The interactive desktop character can be dragged and roam along screen edges"
                    .into(),
            );
        }
    }
    OverlayDiagnostics {
        tier,
        overlay_requested: route.enabled,
        overlay_open: false,
        movement_supported,
        mouse_passthrough: false,
        paused: false,
        reason,
        display_count: cx.displays().len(),
        geometry_snapshot_count: 0,
        simulation_steps: 0,
        native_moves: 0,
    }
}

fn primary_desktop_display(cx: &App) -> Option<DesktopDisplay> {
    let primary_id = cx.primary_display().map(|display| display.id());
    let metrics = cx.global_display_metrics();
    let (id, bounds, scale_factor) = primary_id
        .and_then(|primary_id| metrics.iter().copied().find(|(id, _, _)| *id == primary_id))
        .or_else(|| metrics.first().copied())?;
    let bounds = desktop_coordinate_bounds(bounds, scale_factor);
    Some(DesktopDisplay {
        id: Some(id),
        bounds,
        work_area: bounds,
        scale_factor,
    })
}

fn desktop_displays(cx: &App) -> Vec<DesktopDisplay> {
    cx.global_display_metrics()
        .into_iter()
        .map(|(id, bounds, scale_factor)| {
            let bounds = desktop_coordinate_bounds(bounds, scale_factor);
            DesktopDisplay {
                id: Some(id),
                bounds,
                work_area: bounds,
                scale_factor,
            }
        })
        .collect()
}

fn logical_display_bounds(cx: &App, id: Option<gpui::DisplayId>) -> Option<Bounds<Pixels>> {
    let id = id?;
    cx.global_display_bounds()
        .into_iter()
        .find_map(|(candidate, bounds)| (candidate == id).then_some(bounds))
}

fn safe_corner(bounds: Bounds<Pixels>, extent: f32) -> Point<Pixels> {
    gpui::point(
        px((f32::from(bounds.right()) - extent - STATIC_MARGIN).max(f32::from(bounds.origin.x))),
        px((f32::from(bounds.bottom()) - extent - STATIC_MARGIN).max(f32::from(bounds.origin.y))),
    )
}

fn drag_origin(
    window_origin: Point<Pixels>,
    pointer_position: Point<Pixels>,
    grab_offset: Point<Pixels>,
    pointer_scale: f32,
) -> Point<Pixels> {
    gpui::point(
        window_origin.x + (pointer_position.x - grab_offset.x) * pointer_scale,
        window_origin.y + (pointer_position.y - grab_offset.y) * pointer_scale,
    )
}

#[cfg(target_os = "windows")]
fn desktop_coordinate_bounds(bounds: Bounds<Pixels>, scale_factor: f32) -> Bounds<Pixels> {
    scale_desktop_bounds(bounds, scale_factor)
}

#[cfg(any(test, target_os = "windows"))]
fn scale_desktop_bounds(bounds: Bounds<Pixels>, scale_factor: f32) -> Bounds<Pixels> {
    Bounds {
        origin: gpui::point(
            bounds.origin.x * scale_factor,
            bounds.origin.y * scale_factor,
        ),
        size: gpui::size(
            bounds.size.width * scale_factor,
            bounds.size.height * scale_factor,
        ),
    }
}

#[cfg(not(target_os = "windows"))]
fn desktop_coordinate_bounds(bounds: Bounds<Pixels>, _scale_factor: f32) -> Bounds<Pixels> {
    bounds
}

#[cfg(target_os = "windows")]
fn desktop_coordinate_extent(logical_extent: f32, scale_factor: f32) -> f32 {
    logical_extent * scale_factor
}

#[cfg(not(target_os = "windows"))]
fn desktop_coordinate_extent(logical_extent: f32, _scale_factor: f32) -> f32 {
    logical_extent
}

#[cfg(target_os = "windows")]
fn desktop_pointer_scale(window_scale_factor: f32) -> f32 {
    window_scale_factor
}

#[cfg(not(target_os = "windows"))]
fn desktop_pointer_scale(_window_scale_factor: f32) -> f32 {
    1.0
}

fn clamp_overlay_origin(
    origin: Point<Pixels>,
    work_area: Bounds<Pixels>,
    extent: f32,
) -> Point<Pixels> {
    let min_x = f32::from(work_area.origin.x);
    let min_y = f32::from(work_area.origin.y);
    let max_x = (f32::from(work_area.right()) - extent).max(min_x);
    let max_y = (f32::from(work_area.bottom()) - extent).max(min_y);
    gpui::point(
        px(f32::from(origin.x).clamp(min_x, max_x)),
        px(f32::from(origin.y).clamp(min_y, max_y)),
    )
}

fn display_for_overlay_origin(
    displays: &[DesktopDisplay],
    origin: Point<Pixels>,
    extent: f32,
) -> Option<DesktopDisplay> {
    let center = gpui::point(origin.x + px(extent * 0.5), origin.y + px(extent * 0.5));
    displays
        .iter()
        .copied()
        .find(|display| display.work_area.contains(&center))
        .or_else(|| {
            displays.iter().copied().min_by(|a, b| {
                let distance = |display: &DesktopDisplay| {
                    let display_center = display.work_area.center();
                    let dx = f32::from(display_center.x) - f32::from(center.x);
                    let dy = f32::from(display_center.y) - f32::from(center.y);
                    dx * dx + dy * dy
                };
                distance(a).total_cmp(&distance(b))
            })
        })
}

fn click_reaction_target(
    position: Point2,
    work_area: Bounds<Pixels>,
    extent: f32,
    double_click: bool,
) -> Point2 {
    let min_x = f32::from(work_area.origin.x);
    let max_x = (f32::from(work_area.right()) - extent).max(min_x);
    let midpoint = (min_x + max_x) * 0.5;
    let x = if double_click {
        if position.x < midpoint {
            max_x
        } else {
            min_x
        }
    } else if position.x < midpoint {
        (position.x + CLICK_NUDGE).min(max_x)
    } else {
        (position.x - CLICK_NUDGE).max(min_x)
    };
    let min_y = f32::from(work_area.origin.y);
    let max_y = (f32::from(work_area.bottom()) - extent).max(min_y);
    Point2::new(x, position.y.clamp(min_y, max_y))
}

fn window_top_target(
    window: Bounds<Pixels>,
    work_area: Bounds<Pixels>,
    extent: f32,
) -> Option<Point2> {
    let usable_width = f32::from(window.size.width).min(f32::from(work_area.size.width));
    if usable_width < 80.0 || f32::from(window.size.height) < 40.0 {
        return None;
    }
    let min_x = f32::from(work_area.origin.x);
    let max_x = (f32::from(work_area.right()) - extent).max(min_x);
    let x = (f32::from(window.center().x) - extent * 0.5).clamp(min_x, max_x);
    let min_y = f32::from(work_area.origin.y);
    let max_y = (f32::from(work_area.bottom()) - extent).max(min_y);
    let y = (f32::from(window.origin.y) - extent + 18.0).clamp(min_y, max_y);
    Some(Point2::new(x, y))
}

fn to_point2(point: Point<Pixels>) -> Point2 {
    Point2::new(f32::from(point.x), f32::from(point.y))
}

fn from_point2(point: Point2) -> Point<Pixels> {
    gpui::point(px(point.x), px(point.y))
}

fn to_rect(bounds: Bounds<Pixels>, extent: f32) -> Rect {
    Rect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        (f32::from(bounds.size.width) - extent).max(1.0),
        (f32::from(bounds.size.height) - extent).max(1.0),
    )
}

fn display_label(id: Option<gpui::DisplayId>) -> String {
    id.map(|id| format!("{id:?}"))
        .unwrap_or_else(|| "primary".to_string())
}

fn character_idle_asset(character: CompanionCharacterId) -> &'static str {
    match character {
        CompanionCharacterId::None | CompanionCharacterId::Clippy => "characters/clippy/idle.png",
        CompanionCharacterId::Shelly => "characters/shelly/idle.png",
        CompanionCharacterId::Spark => "characters/spark/idle.png",
        CompanionCharacterId::Byte => "characters/byte/idle.png",
        CompanionCharacterId::Orbit => "characters/orbit/idle.png",
        CompanionCharacterId::Nox => "characters/nox/idle.png",
    }
}

fn character_render_size(scale: CompanionScale) -> f32 {
    match scale {
        CompanionScale::Small => 120.0,
        CompanionScale::Medium => 160.0,
        CompanionScale::Large => 200.0,
    }
}

fn character_window_size(scale: CompanionScale) -> f32 {
    character_render_size(scale)
}

fn character_visual_asset(
    character: CompanionCharacterId,
    state: CharacterVisualState,
) -> &'static str {
    match state {
        CharacterVisualState::Idle => character_idle_asset(character),
        CharacterVisualState::Dragging => match character {
            CompanionCharacterId::None | CompanionCharacterId::Clippy => {
                "characters/clippy/listening.svg"
            }
            CompanionCharacterId::Shelly => "characters/shelly/listening.svg",
            CompanionCharacterId::Spark => "characters/spark/listening.svg",
            CompanionCharacterId::Byte => "characters/byte/listening.svg",
            CompanionCharacterId::Orbit => "characters/orbit/listening.svg",
            CompanionCharacterId::Nox => "characters/nox/listening.svg",
        },
        CharacterVisualState::Reacting => match character {
            CompanionCharacterId::None | CompanionCharacterId::Clippy => {
                "characters/clippy/success.svg"
            }
            CompanionCharacterId::Shelly => "characters/shelly/success.svg",
            CompanionCharacterId::Spark => "characters/spark/success.svg",
            CompanionCharacterId::Byte => "characters/byte/success.svg",
            CompanionCharacterId::Orbit => "characters/orbit/success.svg",
            CompanionCharacterId::Nox => "characters/nox/success.svg",
        },
    }
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    gpui::guess_compositor() == "Wayland"
}

#[cfg(not(target_os = "linux"))]
fn is_wayland_session() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn is_x11_session() -> bool {
    gpui::guess_compositor() == "X11"
}

#[cfg(not(target_os = "linux"))]
fn is_x11_session() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(enabled: bool) -> ClippyConfig {
        let mut config = ClippyConfig::default();
        config.appearance.desktop.enabled = enabled;
        config
    }

    fn display() -> DesktopDisplay {
        DesktopDisplay {
            id: None,
            bounds: Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(800.0), px(600.0)),
            },
            work_area: Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(800.0), px(600.0)),
            },
            scale_factor: 1.0,
        }
    }

    // SDTEST-1478
    #[test]
    fn runtime_route_requires_enabled_character() {
        assert!(!runtime_route(&config(false)).enabled);
        assert!(runtime_route(&config(true)).enabled);
        let mut hidden = config(true);
        hidden.appearance.character = "none".to_string();
        assert!(!runtime_route(&hidden).enabled);
    }

    // SDTEST-1479
    #[test]
    fn runtime_uses_core_simulation_and_clamps_after_monitor_removal() {
        let mut sim =
            RuntimeSimulation::new(display(), character_window_size(CompanionScale::Medium));
        sim.simulation.position = Point2::new(2000.0, 2000.0);
        let smaller = DesktopDisplay {
            bounds: Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(500.0), px(400.0)),
            },
            work_area: Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(500.0), px(400.0)),
            },
            ..display()
        };
        sim.update_display(smaller);
        assert!(sim.bounds().contains(sim.simulation.position));
    }

    // SDTEST-1480
    #[test]
    fn paused_and_reduced_motion_request_no_continuous_frames() {
        let mut sim =
            RuntimeSimulation::new(display(), character_window_size(CompanionScale::Medium));
        sim.playful_target();
        assert!(sim.moving(false));
        assert!(!sim.moving(true));
        sim.pause();
        assert!(!sim.moving(false));
    }

    // SDTEST-1481
    #[test]
    fn character_assets_route_to_existing_pngs() {
        assert_eq!(
            character_idle_asset(CompanionCharacterId::Clippy),
            "characters/clippy/idle.png"
        );
        assert_eq!(
            character_idle_asset(CompanionCharacterId::Nox),
            "characters/nox/idle.png"
        );
        assert_eq!(
            character_visual_asset(CompanionCharacterId::Orbit, CharacterVisualState::Dragging),
            "characters/orbit/listening.svg"
        );
        assert_eq!(
            character_visual_asset(CompanionCharacterId::Nox, CharacterVisualState::Reacting),
            "characters/nox/success.svg"
        );
    }

    // SDTEST-1485
    #[test]
    fn external_window_target_perches_above_the_window_inside_the_work_area() {
        let work_area = display().work_area;
        let window = Bounds {
            origin: gpui::point(px(200.0), px(260.0)),
            size: gpui::size(px(420.0), px(240.0)),
        };
        let extent = character_window_size(CompanionScale::Medium);
        let target =
            window_top_target(window, work_area, extent).expect("eligible external window");
        assert!(to_rect(work_area, extent).contains(target));
        assert!(target.y < f32::from(window.origin.y));
        assert!(window_top_target(
            Bounds {
                size: gpui::size(px(20.0), px(20.0)),
                ..window
            },
            work_area,
            extent
        )
        .is_none());
    }

    // SDTEST-1486
    #[test]
    fn frame_elapsed_uses_real_refresh_delta_after_the_first_frame() {
        let first = Instant::now();
        assert_eq!(frame_elapsed_millis(None, first), STEP.as_millis() as u64);
        assert_eq!(
            frame_elapsed_millis(Some(first), first + Duration::from_millis(16)),
            16
        );
        assert_eq!(frame_elapsed_millis(Some(first), first), 0);

        let extent = character_window_size(CompanionScale::Medium);
        let mut sim = RuntimeSimulation::new(display(), extent);
        let start = sim.simulation.position;
        sim.simulation
            .set_target(Point2::new(start.x - 100.0, start.y));
        assert!(!sim.step_capped(16));
        assert_eq!(sim.pending_ms, 16);
        assert!(sim.step_capped(17));
        assert_eq!(sim.pending_ms, 0);
    }

    // SDTEST-1491
    #[test]
    fn user_drag_preserves_grab_offset_and_routes_across_display_bounds() {
        assert_eq!(character_window_size(CompanionScale::Small), 120.0);
        assert_eq!(character_window_size(CompanionScale::Large), 200.0);
        let origin = drag_origin(
            gpui::point(px(100.0), px(200.0)),
            gpui::point(px(60.0), px(70.0)),
            gpui::point(px(20.0), px(30.0)),
            1.0,
        );
        assert_eq!(origin, gpui::point(px(140.0), px(240.0)));
        assert_eq!(
            drag_origin(
                gpui::point(px(100.0), px(200.0)),
                gpui::point(px(60.0), px(70.0)),
                gpui::point(px(20.0), px(30.0)),
                1.5,
            ),
            gpui::point(px(160.0), px(260.0))
        );

        assert_eq!(
            scale_desktop_bounds(
                Bounds {
                    origin: gpui::point(px(1280.0), px(-80.0)),
                    size: gpui::size(px(1280.0), px(720.0)),
                },
                1.5,
            ),
            Bounds {
                origin: gpui::point(px(1920.0), px(-120.0)),
                size: gpui::size(px(1920.0), px(1080.0)),
            }
        );

        let left = display();
        let right = DesktopDisplay {
            bounds: Bounds {
                origin: gpui::point(px(800.0), px(-120.0)),
                size: gpui::size(px(900.0), px(720.0)),
            },
            work_area: Bounds {
                origin: gpui::point(px(800.0), px(-120.0)),
                size: gpui::size(px(900.0), px(680.0)),
            },
            ..display()
        };
        let extent = character_window_size(CompanionScale::Medium);
        let selected =
            display_for_overlay_origin(&[left, right], gpui::point(px(920.0), px(40.0)), extent)
                .expect("second display");
        assert_eq!(selected.work_area, right.work_area);
        assert_eq!(
            clamp_overlay_origin(gpui::point(px(1800.0), px(-300.0)), right.work_area, extent),
            gpui::point(px(1540.0), px(-120.0))
        );
    }

    // SDTEST-1492
    #[test]
    fn clicks_start_bounded_playful_reactions_from_the_current_position() {
        let extent = character_window_size(CompanionScale::Medium);
        let mut sim = RuntimeSimulation::new(display(), extent);
        sim.simulation.position = Point2::new(100.0, 300.0);
        assert!(sim.react_to_click(1, extent));
        assert_eq!(sim.simulation.state, CharacterSimulationState::Jumping);
        assert_eq!(sim.simulation.target, Some(Point2::new(212.0, 300.0)));

        sim.simulation.position = Point2::new(100.0, 300.0);
        assert!(sim.react_to_click(2, extent));
        assert_eq!(sim.simulation.target, Some(Point2::new(640.0, 300.0)));
        assert!(sim.moving(false));
    }
}
