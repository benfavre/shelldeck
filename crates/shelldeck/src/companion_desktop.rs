use std::time::{Duration, Instant};

use gpui::{
    div, img, prelude::*, px, AnyWindowHandle, App, AppContext, Bounds, Context, Entity,
    IntoElement, ObjectFit, Pixels, Point, Render, Size, Window, WindowBounds, WindowDecorations,
    WindowHandle, WindowKind, WindowOptions,
};
use shelldeck_core::ai::{
    ClippyConfig, CompanionCharacterId, CompanionMotionPreference, CompanionScale,
    DesktopCompanionMovement,
};
use shelldeck_core::companion::geometry::{Point2, Rect};
use shelldeck_core::companion::simulation::{
    AnimationFramePolicy, CharacterSimulation, CharacterSimulationState,
};

const OVERLAY_SIZE: f32 = 224.0;
const STEP: Duration = Duration::from_millis(33);
const STATIC_MARGIN: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionRuntimeCommand {
    Pause,
    ReturnToDock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayCapabilityTier {
    FullRoaming,
    ScreenEdgeOnly,
    DockOnly,
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
    pub dock_only: bool,
}

pub fn runtime_route(config: &ClippyConfig) -> RuntimeRoute {
    let requested_desktop = config.appearance.desktop.enabled
        && config.appearance.character_id() != CompanionCharacterId::None;
    RuntimeRoute {
        enabled: requested_desktop,
        requested_desktop,
        dock_only: !requested_desktop,
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
    steps: u64,
}

impl RuntimeSimulation {
    fn new(display: DesktopDisplay) -> Self {
        let position = safe_corner(display.work_area);
        Self {
            simulation: CharacterSimulation::new(display_label(display.id), to_point2(position)),
            display,
            steps: 0,
        }
    }

    fn update_display(&mut self, display: DesktopDisplay) {
        self.display = display;
        self.simulation.display_id = display_label(display.id);
        self.simulation.position = self.bounds().clamp_point(self.simulation.position);
    }

    fn return_to_dock(&mut self) {
        self.simulation.state = CharacterSimulationState::ReturningToDock;
        self.simulation
            .set_target(to_point2(safe_corner(self.display.work_area)));
    }

    fn pause(&mut self) {
        self.simulation.target = None;
        self.simulation.state = CharacterSimulationState::Resting;
    }

    fn playful_target(&mut self) {
        if !self.simulation.can_start_action() {
            return;
        }
        let bounds = self.display.work_area;
        let left = f32::from(bounds.origin.x) + STATIC_MARGIN;
        let right = f32::from(bounds.right()) - OVERLAY_SIZE - STATIC_MARGIN;
        let floor = f32::from(bounds.bottom()) - OVERLAY_SIZE - STATIC_MARGIN;
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
        let Some(target) = window_top_target(window, self.display.work_area) else {
            return false;
        };
        self.simulation.set_target(target);
        self.simulation.remember_action("window-climb");
        true
    }

    fn step_capped(&mut self, elapsed_ms: u64) -> bool {
        let before = self.simulation.position;
        let steps = self.simulation.step_capped(elapsed_ms, self.bounds());
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
        to_rect(self.display.work_area)
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
                tier: OverlayCapabilityTier::DockOnly,
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
            && self.diagnostics.tier != OverlayCapabilityTier::DockOnly
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
            CompanionRuntimeCommand::ReturnToDock => {
                self.ensure_overlay(runtime_entity, main_window, cx);
                if let Some(sim) = &mut self.simulation {
                    sim.return_to_dock();
                }
                self.request_frame(cx);
            }
        }
    }

    pub fn diagnostics(&self) -> &OverlayDiagnostics {
        &self.diagnostics
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
                (f32::from(next.work_area.bottom()) - OVERLAY_SIZE - STATIC_MARGIN)
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
        if self.overlay.is_some() || self.diagnostics.tier == OverlayCapabilityTier::DockOnly {
            return;
        }
        let Some(display) = primary_desktop_display(cx) else {
            self.diagnostics.reason =
                Some("No display is available for the desktop character".into());
            return;
        };
        self.simulation = Some(RuntimeSimulation::new(display));
        if let Some(sim) = &mut self.simulation {
            if !matches!(
                self.config.appearance.motion,
                CompanionMotionPreference::Off | CompanionMotionPreference::Reduced
            ) && self.config.appearance.desktop.movement != DesktopCompanionMovement::Still
            {
                sim.playful_target();
            }
        }
        let bounds = Bounds {
            origin: safe_corner(display.work_area),
            size: Size {
                width: px(OVERLAY_SIZE),
                height: px(OVERLAY_SIZE),
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
            mouse_passthrough: true,
            focus: false,
            show: true,
            app_id: Some("shelldeck-character".to_string()),
            ..Default::default()
        };
        let asset_path = character_idle_asset(self.config.appearance.character_id());
        let render_size = character_render_size(self.config.appearance.scale);
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| {
                CharacterOverlayView::new(main_window, asset_path, render_size, runtime_entity)
            })
        }) {
            Ok(handle) => {
                self.overlay = Some(handle);
                self.diagnostics.overlay_open = true;
                self.diagnostics.mouse_passthrough = true;
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
                self.diagnostics.tier = OverlayCapabilityTier::DockOnly;
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
    asset_path: &'static str,
    render_size: f32,
    main_window: AnyWindowHandle,
    frame_scheduled: bool,
}

impl CharacterOverlayView {
    fn new(
        main_window: AnyWindowHandle,
        asset_path: &'static str,
        render_size: f32,
        runtime: Entity<DesktopCharacterRuntime>,
    ) -> Self {
        Self {
            runtime,
            asset_path,
            render_size,
            main_window,
            frame_scheduled: false,
        }
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.main_window;
        div().size_full().bg(gpui::transparent_black()).child(
            div()
                .absolute()
                .right_1()
                .bottom_1()
                .w(px(self.render_size))
                .h(px(self.render_size))
                .child(
                    img(self.asset_path)
                        .w_full()
                        .h_full()
                        .object_fit(ObjectFit::Contain),
                ),
        )
    }
}

fn detect_capabilities(cx: &App, route: RuntimeRoute) -> OverlayDiagnostics {
    let mut tier = OverlayCapabilityTier::DockOnly;
    let mut reason = None;
    let mut movement_supported = false;
    if route.enabled {
        if is_wayland_session() {
            reason = Some("Wayland does not expose reliable top-level positioning or external-window geometry, using Dock-only fallback".into());
        } else if is_x11_session() || cfg!(target_os = "windows") || cfg!(target_os = "macos") {
            tier = OverlayCapabilityTier::ScreenEdgeOnly;
            movement_supported = true;
            reason = Some(
                "Window climbing is disabled; the desktop character remains on screen edges".into(),
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
    let global_bounds = cx.global_display_bounds();
    let (id, bounds) = primary_id
        .and_then(|primary_id| {
            global_bounds
                .iter()
                .copied()
                .find(|(id, _)| *id == primary_id)
        })
        .or_else(|| global_bounds.first().copied())?;
    Some(DesktopDisplay {
        id: Some(id),
        bounds,
        work_area: bounds,
        scale_factor: 1.0,
    })
}

fn desktop_displays(cx: &App) -> Vec<DesktopDisplay> {
    cx.global_display_bounds()
        .into_iter()
        .map(|(id, bounds)| DesktopDisplay {
            id: Some(id),
            bounds,
            work_area: bounds,
            scale_factor: 1.0,
        })
        .collect()
}

fn safe_corner(bounds: Bounds<Pixels>) -> Point<Pixels> {
    gpui::point(
        px((f32::from(bounds.right()) - OVERLAY_SIZE - STATIC_MARGIN)
            .max(f32::from(bounds.origin.x))),
        px((f32::from(bounds.bottom()) - OVERLAY_SIZE - STATIC_MARGIN)
            .max(f32::from(bounds.origin.y))),
    )
}

fn window_top_target(window: Bounds<Pixels>, work_area: Bounds<Pixels>) -> Option<Point2> {
    let usable_width = f32::from(window.size.width).min(f32::from(work_area.size.width));
    if usable_width < 80.0 || f32::from(window.size.height) < 40.0 {
        return None;
    }
    let min_x = f32::from(work_area.origin.x);
    let max_x = (f32::from(work_area.right()) - OVERLAY_SIZE).max(min_x);
    let x = (f32::from(window.center().x) - OVERLAY_SIZE * 0.5).clamp(min_x, max_x);
    let min_y = f32::from(work_area.origin.y);
    let max_y = (f32::from(work_area.bottom()) - OVERLAY_SIZE).max(min_y);
    let y = (f32::from(window.origin.y) - OVERLAY_SIZE + 18.0).clamp(min_y, max_y);
    Some(Point2::new(x, y))
}

fn to_point2(point: Point<Pixels>) -> Point2 {
    Point2::new(f32::from(point.x), f32::from(point.y))
}

fn from_point2(point: Point2) -> Point<Pixels> {
    gpui::point(px(point.x), px(point.y))
}

fn to_rect(bounds: Bounds<Pixels>) -> Rect {
    Rect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        (f32::from(bounds.size.width) - OVERLAY_SIZE).max(1.0),
        (f32::from(bounds.size.height) - OVERLAY_SIZE).max(1.0),
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

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s.eq_ignore_ascii_case("wayland"))
}

#[cfg(not(target_os = "linux"))]
fn is_wayland_session() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn is_x11_session() -> bool {
    std::env::var_os("DISPLAY").is_some()
        && !std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s.eq_ignore_ascii_case("wayland"))
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

    // SDTEST-1439
    #[test]
    fn runtime_route_requires_enabled_character() {
        assert!(!runtime_route(&config(false)).enabled);
        assert!(runtime_route(&config(true)).enabled);
        let mut hidden = config(true);
        hidden.appearance.character = "none".to_string();
        assert!(!runtime_route(&hidden).enabled);
    }

    // SDTEST-1440
    #[test]
    fn runtime_uses_core_simulation_and_clamps_after_monitor_removal() {
        let mut sim = RuntimeSimulation::new(display());
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

    // SDTEST-1441
    #[test]
    fn paused_and_reduced_motion_request_no_continuous_frames() {
        let mut sim = RuntimeSimulation::new(display());
        sim.playful_target();
        assert!(sim.moving(false));
        assert!(!sim.moving(true));
        sim.pause();
        assert!(!sim.moving(false));
    }

    // SDTEST-1442
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
    }

    // SDTEST-1446
    #[test]
    fn external_window_target_perches_above_the_window_inside_the_work_area() {
        let work_area = display().work_area;
        let window = Bounds {
            origin: gpui::point(px(200.0), px(260.0)),
            size: gpui::size(px(420.0), px(240.0)),
        };
        let target = window_top_target(window, work_area).expect("eligible external window");
        assert!(to_rect(work_area).contains(target));
        assert!(target.y < f32::from(window.origin.y));
        assert!(window_top_target(
            Bounds {
                size: gpui::size(px(20.0), px(20.0)),
                ..window
            },
            work_area
        )
        .is_none());
    }

    // SDTEST-1447
    #[test]
    fn frame_elapsed_uses_real_refresh_delta_after_the_first_frame() {
        let first = Instant::now();
        assert_eq!(frame_elapsed_millis(None, first), STEP.as_millis() as u64);
        assert_eq!(
            frame_elapsed_millis(Some(first), first + Duration::from_millis(16)),
            16
        );
        assert_eq!(frame_elapsed_millis(Some(first), first), 0);
    }
}
