use std::time::{Duration, Instant};

use gpui::{
    div, img, px, AnyWindowHandle, App, Bounds, Context, Entity, IntoElement, ObjectFit, Pixels,
    Point, Render, Size, Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind,
    WindowOptions,
};
use shelldeck_core::companion::geometry::{Point2, Rect};
use shelldeck_core::companion::simulation::{
    AnimationFramePolicy, CharacterSimulation, CharacterSimulationState,
};
use shelldeck_core::config::app_config::{
    CompanionCharacter, CompanionCharacterMotion, CompanionCharacterPresence, CompanionConfig,
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

pub fn runtime_route(config: &CompanionConfig) -> RuntimeRoute {
    let requested_desktop = config.desktop_character_enabled
        && matches!(
            config.character_presence,
            CompanionCharacterPresence::Desktop
        );
    RuntimeRoute {
        enabled: requested_desktop,
        requested_desktop,
        dock_only: !requested_desktop
            || matches!(
                config.character_presence,
                CompanionCharacterPresence::DockOnly | CompanionCharacterPresence::Hidden
            ),
    }
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
        if !self.simulation.can_start_action()
            || self.simulation.action_on_cooldown("screen-edge-hop")
        {
            return;
        }
        let bounds = self.display.work_area;
        let left = bounds.origin.x.0 + STATIC_MARGIN;
        let right = bounds.right().0 - OVERLAY_SIZE - STATIC_MARGIN;
        let floor = bounds.bottom().0 - OVERLAY_SIZE - STATIC_MARGIN;
        let next_x = if self.simulation.position.x < (left + right) * 0.5 {
            right
        } else {
            left
        };
        self.simulation
            .set_target(Point2::new(next_x, floor.max(bounds.origin.y.0)));
        self.simulation.remember_action("screen-edge-hop");
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
    config: CompanionConfig,
    overlay: Option<WindowHandle<CharacterOverlayView>>,
    diagnostics: OverlayDiagnostics,
    simulation: Option<RuntimeSimulation>,
    last_tick: Option<Instant>,
}

impl DesktopCharacterRuntime {
    pub fn new(config: CompanionConfig) -> Self {
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
        }
    }

    pub fn apply_config(
        &mut self,
        runtime_entity: Entity<Self>,
        config: CompanionConfig,
        main_window: AnyWindowHandle,
        cx: &mut App,
    ) {
        self.config = config;
        self.diagnostics = detect_capabilities(cx, runtime_route(&self.config));
        if self.diagnostics.overlay_requested
            && self.diagnostics.tier != OverlayCapabilityTier::DockOnly
        {
            self.ensure_overlay(runtime_entity, main_window, cx);
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
                    self.ensure_overlay(runtime_entity, main_window, cx);
                    self.request_frame(cx);
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
                self.config.character_motion,
                CompanionCharacterMotion::Off | CompanionCharacterMotion::Reduced
            ) {
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
        let paused = self.diagnostics.paused;
        let asset_path = character_idle_asset(self.config.character);
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| CharacterOverlayView::new(main_window, paused, asset_path, runtime_entity))
        }) {
            Ok(handle) => {
                self.overlay = Some(handle);
                self.diagnostics.overlay_open = true;
                self.diagnostics.mouse_passthrough = true;
                self.diagnostics.display_count = cx.displays().len();
                self.request_frame(cx);
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
        self.diagnostics.overlay_open = false;
    }

    fn request_frame(&self, cx: &mut App) {
        if let Some(handle) = self.overlay {
            let _ = handle.update(cx, |_view, window, _cx| window.request_animation_frame());
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
            self.config.character_motion,
            CompanionCharacterMotion::Reduced | CompanionCharacterMotion::Off
        );
        if self.diagnostics.paused || reduced_motion {
            if let Some(sim) = &mut self.simulation {
                sim.pause();
            }
            return self.frame_state();
        }
        if let Some(sim) = &mut self.simulation {
            if let Some(display) = primary_desktop_display(cx) {
                sim.update_display(display);
                self.diagnostics.display_count = cx.displays().len();
            }
            let now = Instant::now();
            let elapsed = self
                .last_tick
                .map(|last| now.saturating_duration_since(last))
                .unwrap_or(STEP);
            self.last_tick = Some(now);
            let moved = sim.step_capped(elapsed.as_millis().max(STEP.as_millis()) as u64);
            self.diagnostics.simulation_steps = sim.steps;
            if moved {
                self.diagnostics.native_moves += 1;
            }
        }
        self.frame_state()
    }

    fn frame_state(&self) -> CharacterFrameState {
        let reduced_motion = matches!(
            self.config.character_motion,
            CompanionCharacterMotion::Reduced | CompanionCharacterMotion::Off
        );
        CharacterFrameState {
            position: self.simulation.as_ref().map(|sim| sim.position()),
            moving: self
                .simulation
                .as_ref()
                .is_some_and(|sim| sim.moving(reduced_motion)),
            paused: self.diagnostics.paused,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CharacterFrameState {
    position: Option<Point<Pixels>>,
    moving: bool,
    paused: bool,
}

pub struct CharacterOverlayView {
    paused: bool,
    runtime: Entity<DesktopCharacterRuntime>,
    asset_path: &'static str,
    main_window: AnyWindowHandle,
}

impl CharacterOverlayView {
    fn new(
        main_window: AnyWindowHandle,
        paused: bool,
        asset_path: &'static str,
        runtime: Entity<DesktopCharacterRuntime>,
    ) -> Self {
        Self {
            paused,
            runtime,
            asset_path,
            main_window,
        }
    }
}

impl Render for CharacterOverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.runtime.update(cx, |runtime, cx| runtime.on_frame(cx));
        self.paused = state.paused;
        if let Some(position) = state.position {
            if let Err(error) = window.set_window_origin(position) {
                tracing::warn!(error = %error, "desktop character native movement failed");
                self.runtime.update(cx, |runtime, _cx| {
                    runtime.record_movement_error(error.to_string())
                });
            }
        }
        if state.moving {
            window.request_animation_frame();
        }
        let _ = self.main_window;
        div().size_full().bg(gpui::transparent_black()).child(
            div()
                .absolute()
                .right_1()
                .bottom_1()
                .w(px(160.0))
                .h(px(160.0))
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
            reason = Some("Desktop character roaming is limited to screen-edge movement until external-window geometry providers are enabled".into());
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
    let display = cx
        .primary_display()
        .or_else(|| cx.displays().first().cloned())?;
    Some(DesktopDisplay {
        id: Some(display.id()),
        bounds: display.bounds(),
        work_area: display.bounds(),
        scale_factor: display.scale_factor(),
    })
}

fn safe_corner(bounds: Bounds<Pixels>) -> Point<Pixels> {
    gpui::point(
        px((bounds.right().0 - OVERLAY_SIZE - STATIC_MARGIN).max(bounds.origin.x.0)),
        px((bounds.bottom().0 - OVERLAY_SIZE - STATIC_MARGIN).max(bounds.origin.y.0)),
    )
}

fn to_point2(point: Point<Pixels>) -> Point2 {
    Point2::new(point.x.0, point.y.0)
}

fn from_point2(point: Point2) -> Point<Pixels> {
    gpui::point(px(point.x), px(point.y))
}

fn to_rect(bounds: Bounds<Pixels>) -> Rect {
    Rect::new(
        bounds.origin.x.0,
        bounds.origin.y.0,
        (bounds.size.width.0 - OVERLAY_SIZE).max(1.0),
        (bounds.size.height.0 - OVERLAY_SIZE).max(1.0),
    )
}

fn display_label(id: Option<gpui::DisplayId>) -> String {
    id.map(|id| format!("{id:?}"))
        .unwrap_or_else(|| "primary".to_string())
}

fn character_idle_asset(character: CompanionCharacter) -> &'static str {
    match character {
        CompanionCharacter::None | CompanionCharacter::Clippy => "characters/clippy/idle.png",
        CompanionCharacter::Shelly => "characters/shelly/idle.png",
        CompanionCharacter::Spark => "characters/spark/idle.png",
        CompanionCharacter::Byte => "characters/byte/idle.png",
        CompanionCharacter::Orbit => "characters/orbit/idle.png",
        CompanionCharacter::Nox => "characters/nox/idle.png",
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

    fn config(presence: CompanionCharacterPresence, enabled: bool) -> CompanionConfig {
        CompanionConfig {
            character_presence: presence,
            desktop_character_enabled: enabled,
            ..Default::default()
        }
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

    #[test]
    fn runtime_route_requires_desktop_presence_and_enable_flag() {
        assert!(!runtime_route(&config(CompanionCharacterPresence::DockOnly, true)).enabled);
        assert!(!runtime_route(&config(CompanionCharacterPresence::Desktop, false)).enabled);
        assert!(runtime_route(&config(CompanionCharacterPresence::Desktop, true)).enabled);
        assert!(!runtime_route(&config(CompanionCharacterPresence::Hidden, true)).enabled);
    }

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

    #[test]
    fn paused_and_reduced_motion_request_no_continuous_frames() {
        let mut sim = RuntimeSimulation::new(display());
        sim.playful_target();
        assert!(sim.moving(false));
        assert!(!sim.moving(true));
        sim.pause();
        assert!(!sim.moving(false));
    }

    #[test]
    fn character_assets_route_to_existing_pngs() {
        assert_eq!(
            character_idle_asset(CompanionCharacter::Clippy),
            "characters/clippy/idle.png"
        );
        assert_eq!(
            character_idle_asset(CompanionCharacter::Nox),
            "characters/nox/idle.png"
        );
    }
}
