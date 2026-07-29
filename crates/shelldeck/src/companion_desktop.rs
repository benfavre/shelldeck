use std::time::{Duration, Instant};

use gpui::{
    div, ease_in_out, img, prelude::*, px, Animation, AnimationExt, AnyWindowHandle, App,
    AppContext, Bounds, Context, Entity, ExternalWindow, ExternalWindowId, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, Render,
    SharedString, Size, Window, WindowBounds, WindowDecorations, WindowHandle, WindowKind,
    WindowOptions,
};
use shelldeck_core::ai::{
    ClippyConfig, CompanionCharacterId, CompanionMotionPreference, CompanionScale,
    DesktopCompanionMovement,
};
use shelldeck_core::companion::geometry::{
    LineSegment, Point2, Rect, SurfaceId, Vector2, WalkableSurface, WalkableSurfaceKind,
};
use shelldeck_core::companion::physics::{
    BodyMode, CompanionBody, PhysicsConfig, StepResult, SurfaceContact,
};
use shelldeck_core::companion::simulation::{
    AnimationFramePolicy, CharacterSimulation, CharacterSimulationState, DeterministicRng,
};

const STEP: Duration = Duration::from_millis(33);
const STATIC_MARGIN: f32 = 24.0;
const DRAG_THRESHOLD: f32 = 4.0;
const CLICK_NUDGE: f32 = 112.0;
const REACTION_DURATION: Duration = Duration::from_millis(650);
const LANDING_DURATION: Duration = Duration::from_millis(320);
const IDLE_FLOURISH_DURATION: Duration = Duration::from_millis(2_800);
const IDLE_FLOURISH_DELAY: Duration = Duration::from_secs(8);
const ATTACHMENT_FOLLOW_INTERVAL: Duration = Duration::from_millis(100);
const DRAG_VELOCITY_SAMPLE_LIMIT: Duration = Duration::from_millis(120);
const DRAG_WINDOW_LIST_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const DRAG_LOCKED_WINDOW_REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const SNAP_VERTICAL_GAP: f32 = 42.0;
const SNAP_MIN_HORIZONTAL_OVERLAP: f32 = 36.0;
const SNAP_PREVIEW_VERTICAL_GAP: f32 = 72.0;
const SNAP_HYSTERESIS_VERTICAL_GAP: f32 = 110.0;
const SNAP_HYSTERESIS_MIN_HORIZONTAL_OVERLAP: f32 = 12.0;
const MAXIMIZED_EDGE_INSET_TOLERANCE: f32 = 96.0;
const MAXIMIZED_COVERAGE_RATIO: f32 = 0.92;

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
    character: CompanionCharacterId,
    rng: DeterministicRng,
    logical_extent: f32,
    extent: f32,
    pending_ms: u64,
    steps: u64,
    body: CompanionBody,
    physics_pending_ms: u64,
}

impl RuntimeSimulation {
    fn new(display: DesktopDisplay, logical_extent: f32, character: CompanionCharacterId) -> Self {
        let extent = desktop_coordinate_extent(logical_extent, display.scale_factor);
        let position = safe_corner(display.work_area, extent);
        let profile = character_personality(character);
        let mut simulation =
            CharacterSimulation::new(display_label(display.id), to_point2(position));
        simulation.config.speed_per_second = profile.base_speed;
        let mut body = CompanionBody::new(to_point2(position), Point2::new(extent, extent));
        body.mode = BodyMode::Sleeping;
        Self {
            simulation,
            display,
            character,
            rng: DeterministicRng::seeded(character_seed(character)),
            logical_extent,
            extent,
            pending_ms: 0,
            steps: 0,
            body,
            physics_pending_ms: 0,
        }
    }

    fn update_display(&mut self, display: DesktopDisplay) {
        self.display = display;
        self.extent = desktop_coordinate_extent(self.logical_extent, display.scale_factor);
        self.simulation.display_id = display_label(display.id);
        self.simulation.position = self.bounds().clamp_point(self.simulation.position);
        self.body.position = self.simulation.position;
        self.body.size = Point2::new(self.extent, self.extent);
    }

    fn return_to_corner(&mut self) {
        self.pending_ms = 0;
        self.physics_pending_ms = 0;
        self.body.position = self.simulation.position;
        self.body.mode = BodyMode::Kinematic;
        self.simulation.config.speed_per_second =
            character_personality(self.character).return_speed;
        self.simulation.state = CharacterSimulationState::ReturningToDock;
        self.simulation
            .set_target(to_point2(safe_corner(self.display.work_area, self.extent)));
        self.simulation.state = CharacterSimulationState::ReturningToDock;
    }

    fn place_from_drag(&mut self, origin: Point<Pixels>, extent: f32) -> Point<Pixels> {
        let origin = clamp_overlay_origin(origin, self.display.work_area, extent);
        self.simulation.position = to_point2(origin);
        self.body.position = self.simulation.position;
        self.body.size = Point2::new(extent, extent);
        self.body.mode = BodyMode::Kinematic;
        self.simulation.target = None;
        self.simulation.state = CharacterSimulationState::Summoned;
        self.pending_ms = 0;
        self.physics_pending_ms = 0;
        origin
    }

    fn release_dynamic(&mut self, velocity: Point2) {
        self.simulation.target = None;
        self.simulation.state = CharacterSimulationState::Flying;
        self.body.position = self.simulation.position;
        self.body.size = Point2::new(self.extent, self.extent);
        self.body
            .release_from_drag(velocity, PhysicsConfig::default());
        self.physics_pending_ms = 0;
    }

    fn snap_body_to_surface(&mut self, surface: &WalkableSurface) {
        self.body.position = self.simulation.position;
        self.body.size = Point2::new(self.extent, self.extent);
        self.body.snap_to_surface(surface);
        self.simulation.position = self.body.position;
        self.simulation.target = None;
        self.simulation.state = CharacterSimulationState::Perched;
        self.pending_ms = 0;
        self.physics_pending_ms = 0;
    }

    fn step_physics_capped(
        &mut self,
        elapsed_ms: u64,
        platforms: &[WalkableSurface],
    ) -> StepResult {
        let step_ms = self.simulation.config.fixed_timestep_ms.max(1);
        let max_steps = u64::from(self.simulation.config.max_catch_up_steps);
        let max_budget_ms = step_ms.saturating_mul(max_steps);
        self.physics_pending_ms = self.physics_pending_ms.saturating_add(elapsed_ms);
        let budget_ms = self.physics_pending_ms.min(max_budget_ms);
        let mut result = self.body.step(
            0.0,
            PhysicsConfig::default(),
            platforms,
            self.physics_bounds(),
        );
        let steps = budget_ms / step_ms;
        for _ in 0..steps {
            result = self.body.step(
                step_ms as f32 / 1000.0,
                PhysicsConfig::default(),
                platforms,
                self.physics_bounds(),
            );
            self.steps = self.steps.saturating_add(1);
        }
        let consumed_ms = steps.saturating_mul(step_ms);
        self.physics_pending_ms = if self.physics_pending_ms > max_budget_ms {
            self.physics_pending_ms % step_ms
        } else {
            self.physics_pending_ms.saturating_sub(consumed_ms)
        };
        self.simulation.position = self.body.position;
        if result.landed {
            self.simulation.state = CharacterSimulationState::Landing;
        }
        result
    }

    fn physics_dynamic(&self) -> bool {
        self.body.mode == BodyMode::Dynamic
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
        let profile = character_personality(self.character);
        self.simulation.config.speed_per_second = if click_count >= 2 {
            profile.double_click_speed
        } else {
            profile.click_speed
        };
        self.simulation.set_target(target);
        self.simulation.state = if profile.airborne || click_count >= 2 {
            CharacterSimulationState::Flying
        } else {
            CharacterSimulationState::Jumping
        };
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
        let Some(target) = self.next_playful_target() else {
            return;
        };
        let profile = character_personality(self.character);
        self.simulation.config.speed_per_second = target.speed;
        self.simulation.set_target(target.point);
        self.simulation.state = if profile.airborne || target.airborne {
            CharacterSimulationState::Flying
        } else {
            CharacterSimulationState::Walking
        };
        self.simulation.remember_action(target.action_id);
    }

    fn next_playful_target(&mut self) -> Option<PlayfulTarget> {
        let bounds = self.display.work_area;
        let profile = character_personality(self.character);
        let candidates = playful_targets(
            self.character,
            bounds,
            self.extent,
            self.simulation.position,
            &mut self.rng,
        );
        let target_count = candidates.len();
        let start_index = self.rng.choose_index(target_count).unwrap_or(0);
        for offset in 0..target_count {
            let index = (start_index + offset) % target_count;
            let candidate = candidates[index];
            if !self.simulation.action_on_cooldown(candidate.action_id) {
                return Some(candidate.with_speed(profile, &mut self.rng));
            }
        }
        candidates
            .get(start_index)
            .copied()
            .map(|candidate| candidate.with_speed(profile, &mut self.rng))
    }

    fn step_capped(&mut self, elapsed_ms: u64) -> StepOutcome {
        let before = self.simulation.position;
        let had_target = self.simulation.target.is_some();
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
        StepOutcome {
            moved: self.simulation.position != before,
            landed: had_target && self.simulation.target.is_none(),
        }
    }

    fn position(&self) -> Point<Pixels> {
        from_point2(self.simulation.position)
    }

    fn moving(&self, reduced_motion: bool) -> bool {
        if !reduced_motion && self.physics_dynamic() {
            return true;
        }
        self.simulation.request_animation_frames(reduced_motion) == AnimationFramePolicy::Continuous
    }

    fn bounds(&self) -> Rect {
        to_rect(self.display.work_area, self.extent)
    }

    fn physics_bounds(&self) -> Rect {
        raw_rect(self.display.work_area)
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
    attachment: Option<WindowAttachment>,
    attachment_timer_scheduled: bool,
    attachment_generation: u64,
    pending_native_move: bool,
    physics_windows: Vec<ExternalWindow>,
    physics_platforms: Vec<WalkableSurface>,
    drag_windows: Vec<ExternalWindow>,
    drag_displays: Vec<DesktopDisplay>,
    drag_snapshot_at: Option<Instant>,
    drag_locked_snapshot_at: Option<Instant>,
    drag_free_position: Option<Point2>,
    drag_snap: Option<SnapCandidate>,
    drag_invalidated_preview_id: Option<ExternalWindowId>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct WindowAttachment {
    id: ExternalWindowId,
    top_edge_offset: f32,
    perched: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DragMoveOutcome {
    position: Point<Pixels>,
    snapped: bool,
    moved: bool,
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
            attachment: None,
            attachment_timer_scheduled: false,
            attachment_generation: 0,
            pending_native_move: false,
            physics_windows: Vec::new(),
            physics_platforms: Vec::new(),
            drag_windows: Vec::new(),
            drag_displays: Vec::new(),
            drag_snapshot_at: None,
            drag_locked_snapshot_at: None,
            drag_free_position: None,
            drag_snap: None,
            drag_invalidated_preview_id: None,
        }
    }

    pub fn apply_config(
        &mut self,
        runtime_entity: Entity<Self>,
        config: ClippyConfig,
        main_window: AnyWindowHandle,
        cx: &mut App,
    ) {
        let was_paused = self.diagnostics.paused;
        let was_reduced = reduced_motion_for_config(&self.config);
        let previous_movement = self.config.appearance.desktop.movement;
        let previous_window_climbing = self.config.appearance.desktop.allow_window_climbing;
        let recreate = self.config.appearance.character_id() != config.appearance.character_id()
            || self.config.appearance.scale != config.appearance.scale;
        self.config = config;
        self.apply_window_climbing_transition(previous_window_climbing);
        let reduced_motion = reduced_motion_for_config(&self.config);
        let motion_policy_changed = was_reduced != reduced_motion
            || previous_movement != self.config.appearance.desktop.movement;
        self.roam_generation = self.roam_generation.wrapping_add(1);
        self.roam_timer_scheduled = false;
        self.cancel_attachment();
        self.clear_drag_snapshot();
        if recreate {
            self.close_overlay(cx);
        }
        self.diagnostics = detect_capabilities(cx, runtime_route(&self.config));
        self.diagnostics.paused = was_paused;
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
            if !recreate && motion_policy_changed {
                if let Some(sim) = &mut self.simulation {
                    if reduced_motion || self.diagnostics.paused {
                        sim.pause();
                    } else {
                        sim.playful_target();
                    }
                }
                self.last_tick = None;
                self.request_frame(cx);
            }
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
                if self.diagnostics.paused {
                    self.cancel_attachment();
                    self.clear_drag_snapshot();
                }
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
                self.cancel_attachment();
                self.clear_drag_snapshot();
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
        self.cancel_attachment();
        self.clear_drag_snapshot();
        self.last_tick = None;
        if let Some(sim) = &mut self.simulation {
            sim.pause();
            sim.simulation.state = CharacterSimulationState::Summoned;
        }
    }

    fn refresh_drag_snapshot(&mut self, cx: &App, now: Instant) {
        if let Some(preview_id) = self.drag_snap.as_ref().map(|candidate| candidate.window.id) {
            if drag_snapshot_refresh_due(
                self.drag_locked_snapshot_at,
                now,
                DRAG_LOCKED_WINDOW_REFRESH_INTERVAL,
            ) {
                if let Some(window) = cx.external_window(preview_id) {
                    if let Some(existing) = self
                        .drag_windows
                        .iter_mut()
                        .find(|existing| existing.id == preview_id)
                    {
                        *existing = window;
                    } else {
                        self.drag_windows.push(window);
                    }
                    self.invalidate_drag_preview_if_geometry_changed(preview_id);
                } else {
                    self.drag_windows.retain(|window| window.id != preview_id);
                    self.drag_snap = None;
                    self.drag_invalidated_preview_id = Some(preview_id);
                }
                self.drag_locked_snapshot_at = Some(now);
                self.diagnostics.geometry_snapshot_count =
                    self.diagnostics.geometry_snapshot_count.saturating_add(1);
            }
        }
        if !drag_snapshot_refresh_due(
            self.drag_snapshot_at,
            now,
            DRAG_WINDOW_LIST_REFRESH_INTERVAL,
        ) {
            return;
        }
        self.drag_windows = cx.visible_external_windows();
        self.drag_displays = desktop_displays(cx);
        self.drag_snapshot_at = Some(now);
        self.drag_locked_snapshot_at = Some(now);
        self.diagnostics.geometry_snapshot_count =
            self.diagnostics.geometry_snapshot_count.saturating_add(1);
        self.diagnostics.display_count = self.drag_displays.len();
    }

    fn invalidate_drag_preview_if_geometry_changed(&mut self, preview_id: ExternalWindowId) {
        let Some(position) = self.drag_free_position else {
            return;
        };
        let Some(logical_extent) = self.simulation.as_ref().map(|sim| sim.logical_extent) else {
            return;
        };
        if choose_snap_window_for_id(
            &self.drag_windows,
            &self.drag_displays,
            position,
            logical_extent,
            preview_id,
            SNAP_HYSTERESIS_VERTICAL_GAP,
            SNAP_HYSTERESIS_MIN_HORIZONTAL_OVERLAP,
        )
        .is_none()
        {
            self.drag_snap = None;
            self.drag_invalidated_preview_id = Some(preview_id);
        }
    }

    fn clear_drag_snapshot(&mut self) {
        self.drag_windows.clear();
        self.drag_displays.clear();
        self.drag_snapshot_at = None;
        self.drag_locked_snapshot_at = None;
        self.drag_free_position = None;
        self.drag_snap = None;
        self.drag_invalidated_preview_id = None;
    }

    fn drag_to(&mut self, origin: Point<Pixels>, cx: &App) -> Option<DragMoveOutcome> {
        self.refresh_drag_snapshot(cx, Instant::now());
        let logical_extent = self.simulation.as_ref()?.logical_extent;
        let current_snap_id = self.drag_snap.as_ref().map(|candidate| candidate.window.id);
        let raw_position = to_point2(origin);
        self.drag_free_position = Some(raw_position);
        let candidate = if self.config.appearance.desktop.allow_window_climbing
            && self.drag_invalidated_preview_id.is_none()
        {
            choose_magnetic_snap_window(
                &self.drag_windows,
                &self.drag_displays,
                raw_position,
                logical_extent,
                current_snap_id,
            )
        } else {
            None
        };
        let display = candidate
            .as_ref()
            .map(|candidate| candidate.display)
            .or_else(|| {
                let extent = self.simulation.as_ref()?.extent;
                display_for_overlay_origin(&self.drag_displays, origin, extent)
            })?;
        let snapped = candidate.is_some();
        let sim = self.simulation.as_mut()?;
        let before = sim.simulation.position;
        sim.update_display(display);
        let desired = candidate
            .as_ref()
            .and_then(|candidate| magnetic_snap_position(candidate, raw_position))
            .unwrap_or(raw_position);
        let position = sim.place_from_drag(from_point2(desired), sim.extent);
        let moved = sim.simulation.position != before;
        self.drag_snap = candidate;
        if moved {
            self.diagnostics.native_moves = self.diagnostics.native_moves.saturating_add(1);
        }
        Some(DragMoveOutcome {
            position,
            snapped,
            moved,
        })
    }

    fn finish_user_drag(
        &mut self,
        release_velocity: Point2,
        runtime_entity: Entity<Self>,
        cx: &mut App,
    ) -> bool {
        let windows = cx.visible_external_windows();
        let displays = desktop_displays(cx);
        self.diagnostics.geometry_snapshot_count =
            self.diagnostics.geometry_snapshot_count.saturating_add(1);
        let lifecycle = self.finish_user_drag_snapshot(release_velocity, &windows, &displays);
        self.last_tick = None;
        match lifecycle {
            ReleaseLifecycle::RequestPhysicsFrame => self.request_frame(cx),
            ReleaseLifecycle::StartAttachmentFollow => {
                self.schedule_attachment_follow(runtime_entity.clone(), cx)
            }
            ReleaseLifecycle::ScheduleRoam => self.schedule_roam(runtime_entity.clone(), cx),
        }
        lifecycle == ReleaseLifecycle::RequestPhysicsFrame
    }

    #[cfg(test)]
    fn finish_user_drag_snapshot_for_test(
        &mut self,
        release_velocity: Point2,
        windows: &[ExternalWindow],
        displays: &[DesktopDisplay],
    ) -> bool {
        self.finish_user_drag_snapshot(release_velocity, windows, displays)
            == ReleaseLifecycle::RequestPhysicsFrame
    }

    #[cfg(test)]
    fn finish_user_drag_lifecycle_for_test(
        &mut self,
        release_velocity: Point2,
        windows: &[ExternalWindow],
        displays: &[DesktopDisplay],
    ) -> ReleaseLifecycle {
        self.finish_user_drag_snapshot(release_velocity, windows, displays)
    }

    fn finish_user_drag_snapshot(
        &mut self,
        release_velocity: Point2,
        windows: &[ExternalWindow],
        displays: &[DesktopDisplay],
    ) -> ReleaseLifecycle {
        let preview_id = self.drag_snap.as_ref().map(|candidate| candidate.window.id);
        let preview_invalidated = self.drag_invalidated_preview_id.is_some();
        let free_position = self
            .drag_free_position
            .or_else(|| self.simulation.as_ref().map(|sim| sim.simulation.position));
        self.clear_drag_snapshot();
        let full_motion = !self.diagnostics.paused && !reduced_motion_for_config(&self.config);
        self.clear_physics_snapshot();
        let Some(sim) = &mut self.simulation else {
            return ReleaseLifecycle::ScheduleRoam;
        };
        sim.simulation.target = None;
        sim.simulation.remember_action("user-drag");
        if let Some(display) = displays.iter().copied().find(|display| {
            display
                .work_area
                .contains(&from_point2(sim.simulation.position))
        }) {
            sim.update_display(display);
        }
        if self.config.appearance.desktop.allow_window_climbing {
            let candidate = match (preview_invalidated, preview_id, free_position) {
                (true, _, _) => None,
                (false, Some(id), Some(position)) => choose_snap_window_for_id(
                    windows,
                    displays,
                    position,
                    sim.logical_extent,
                    id,
                    SNAP_HYSTERESIS_VERTICAL_GAP,
                    SNAP_HYSTERESIS_MIN_HORIZONTAL_OVERLAP,
                ),
                (false, None, Some(position)) => {
                    choose_snap_window(windows, displays, position, sim.logical_extent)
                }
                _ => None,
            };
            if let Some(candidate) = candidate {
                sim.update_display(candidate.display);
                if let Some(position) = free_position.and_then(|position| {
                    perch_origin(
                        candidate.window.bounds,
                        candidate.display.work_area,
                        candidate.extent,
                        position.x,
                        candidate.min_horizontal_overlap,
                    )
                }) {
                    sim.simulation.position = position;
                }
                let surface = window_top_surface(&candidate.window, candidate.generation);
                sim.snap_body_to_surface(&surface);
                self.attachment = Some(WindowAttachment {
                    id: candidate.window.id,
                    top_edge_offset: sim.simulation.position.x
                        - f32::from(candidate.window.bounds.origin.x),
                    perched: true,
                });
                self.attachment_generation = self.attachment_generation.wrapping_add(1);
                self.attachment_timer_scheduled = false;
                return ReleaseLifecycle::StartAttachmentFollow;
            }
        }
        if full_motion {
            self.physics_windows = windows.to_vec();
            self.physics_platforms = if self.config.appearance.desktop.allow_window_climbing {
                physics_platforms(&self.physics_windows, displays)
            } else {
                Vec::new()
            };
            sim.release_dynamic(release_velocity);
            ReleaseLifecycle::RequestPhysicsFrame
        } else {
            let origin = from_point2(sim.simulation.position);
            sim.place_from_drag(origin, sim.extent);
            sim.simulation.state = CharacterSimulationState::Resting;
            sim.body.mode = BodyMode::Sleeping;
            ReleaseLifecycle::ScheduleRoam
        }
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
        self.cancel_attachment();
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
            || self.attachment.is_some()
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
                    runtime.cancel_attachment();
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
        let windows = cx.visible_external_windows();
        self.diagnostics.geometry_snapshot_count =
            self.diagnostics.geometry_snapshot_count.saturating_add(1);
        let Some(sim) = self.simulation.as_ref() else {
            return false;
        };
        let position = sim.simulation.position;
        let displays = desktop_displays(cx);
        let restricted_display =
            (!self.config.appearance.desktop.allow_multi_monitor).then_some(sim.display);
        let Some(window) =
            choose_external_window(&windows, &displays, position, restricted_display)
        else {
            return false;
        };
        let window_id = external_window_id(&window);
        let window_bounds = window.bounds;
        self.begin_attachment(window_id, window_bounds, &displays)
    }

    fn begin_attachment(
        &mut self,
        window_id: ExternalWindowId,
        window_bounds: Bounds<Pixels>,
        displays: &[DesktopDisplay],
    ) -> bool {
        let Some(sim) = &mut self.simulation else {
            return false;
        };
        if let Some(display) = display_for_window(displays, window_bounds) {
            sim.update_display(display);
        }
        let min_overlap =
            desktop_coordinate_distance(SNAP_MIN_HORIZONTAL_OVERLAP, sim.display.scale_factor);
        let Some(target) = window_top_target(
            window_bounds,
            sim.display.work_area,
            sim.extent,
            min_overlap,
        ) else {
            return false;
        };
        sim.simulation.config.speed_per_second = character_personality(sim.character).climb_speed;
        sim.simulation.set_target(target);
        sim.simulation.state = CharacterSimulationState::Climbing;
        sim.simulation.remember_action("window-climb");
        self.attachment = Some(WindowAttachment {
            id: window_id,
            top_edge_offset: target.x - f32::from(window_bounds.origin.x),
            perched: false,
        });
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        self.attachment_timer_scheduled = false;
        true
    }

    fn schedule_attachment_follow(&mut self, runtime_entity: Entity<Self>, cx: &mut App) {
        if self.attachment.is_none() || self.attachment_timer_scheduled {
            return;
        }
        self.attachment_timer_scheduled = true;
        let generation = self.attachment_generation;
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(ATTACHMENT_FOLLOW_INTERVAL)
                .await;
            let _ = cx.update(|cx| {
                runtime_entity.update(cx, |runtime, cx| {
                    if runtime.attachment_generation != generation {
                        return;
                    }
                    runtime.attachment_timer_scheduled = false;
                    let outcome = runtime.follow_attached_window(cx);
                    if outcome.moved {
                        runtime.request_frame(cx);
                    }
                    if outcome.attached {
                        runtime.schedule_attachment_follow(runtime_entity.clone(), cx);
                    } else if outcome.restart_dynamic_frames {
                        runtime.request_frame(cx);
                    } else {
                        runtime.schedule_roam(runtime_entity.clone(), cx);
                    }
                });
            });
        })
        .detach();
    }

    fn follow_attached_window(&mut self, cx: &App) -> AttachmentFollowOutcome {
        let Some(attachment) = self.attachment else {
            return AttachmentFollowOutcome::inactive();
        };
        let window = cx.external_window(attachment.id);
        self.diagnostics.geometry_snapshot_count =
            self.diagnostics.geometry_snapshot_count.saturating_add(1);
        let displays = desktop_displays(cx);
        let Some(window) = window else {
            return if self.detach_missing_attachment() {
                AttachmentFollowOutcome::dynamic_restart()
            } else {
                AttachmentFollowOutcome::inactive()
            };
        };
        self.follow_attached_bounds(window.bounds, &displays)
    }

    #[cfg(test)]
    fn follow_attached_snapshot(
        &mut self,
        windows: &[ExternalWindow],
        displays: &[DesktopDisplay],
    ) -> AttachmentFollowOutcome {
        let Some(attachment) = self.attachment else {
            return AttachmentFollowOutcome::inactive();
        };
        let Some(window) = windows
            .iter()
            .find(|window| external_window_id(window) == attachment.id)
        else {
            return if self.detach_missing_attachment() {
                AttachmentFollowOutcome::dynamic_restart()
            } else {
                AttachmentFollowOutcome::inactive()
            };
        };
        self.follow_attached_bounds(window.bounds, displays)
    }

    fn follow_attached_bounds(
        &mut self,
        window_bounds: Bounds<Pixels>,
        displays: &[DesktopDisplay],
    ) -> AttachmentFollowOutcome {
        let Some(attachment) = self.attachment else {
            return AttachmentFollowOutcome::inactive();
        };
        let Some(sim) = &mut self.simulation else {
            self.cancel_attachment();
            return AttachmentFollowOutcome::inactive();
        };
        if let Some(display) = display_for_window(displays, window_bounds) {
            sim.update_display(display);
            self.diagnostics.display_count = displays.len();
        }
        let Some(target) = attachment_target(
            window_bounds,
            sim.display.work_area,
            sim.extent,
            f32::from(window_bounds.origin.x) + attachment.top_edge_offset,
            desktop_coordinate_distance(SNAP_MIN_HORIZONTAL_OVERLAP, sim.display.scale_factor),
        ) else {
            self.cancel_attachment();
            return AttachmentFollowOutcome::inactive();
        };
        let before = sim.simulation.position;
        if attachment.perched {
            sim.simulation.position = target;
            sim.simulation.target = None;
            sim.simulation.state = CharacterSimulationState::Perched;
            self.pending_native_move |= sim.simulation.position != before;
        } else {
            sim.simulation.set_target(target);
        }
        AttachmentFollowOutcome {
            attached: true,
            moved: sim.simulation.position != before,
            restart_dynamic_frames: false,
        }
    }

    fn cancel_attachment(&mut self) {
        self.attachment = None;
        self.attachment_timer_scheduled = false;
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        self.pending_native_move = false;
    }

    fn clear_physics_snapshot(&mut self) {
        self.physics_windows.clear();
        self.physics_platforms.clear();
    }

    fn apply_window_climbing_transition(&mut self, previous_window_climbing: bool) {
        if previous_window_climbing && !self.config.appearance.desktop.allow_window_climbing {
            self.clear_physics_snapshot();
        }
    }

    fn detach_missing_attachment(&mut self) -> bool {
        self.cancel_attachment();
        self.clear_physics_snapshot();
        if let Some(sim) = &mut self.simulation {
            if !self.diagnostics.paused && !reduced_motion_for_config(&self.config) {
                sim.release_dynamic(Point2::new(0.0, 0.0));
                return true;
            } else {
                sim.simulation.state = CharacterSimulationState::Resting;
                sim.body.mode = BodyMode::Sleeping;
            }
        }
        false
    }

    fn mark_attachment_perched(&mut self) {
        if let Some(attachment) = &mut self.attachment {
            attachment.perched = true;
        }
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
        let character = self.config.appearance.character_id();
        self.simulation = Some(RuntimeSimulation::new(display, window_size, character));
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
        let reduced_motion = reduced_motion_for_config(&self.config);
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| {
                CharacterOverlayView::new(main_window, character, reduced_motion, runtime_entity)
            })
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
        self.cancel_attachment();
        self.clear_drag_snapshot();
        self.diagnostics.overlay_open = false;
    }

    fn request_frame(&self, cx: &mut App) {
        if let Some(handle) = self.overlay {
            let _ = handle.update(cx, |view, window, cx| view.schedule_frame(window, cx));
        }
    }

    fn record_movement_error(&mut self, error: String) {
        self.cancel_attachment();
        self.clear_drag_snapshot();
        self.diagnostics.reason = Some(format!("Native movement failed: {error}"));
        self.diagnostics.paused = true;
        if let Some(sim) = &mut self.simulation {
            sim.pause();
        }
    }

    fn on_frame(&mut self, cx: &mut App) -> CharacterFrameState {
        let reduced_motion = reduced_motion_for_config(&self.config);
        if self.diagnostics.paused || reduced_motion {
            self.cancel_attachment();
            if let Some(sim) = &mut self.simulation {
                sim.pause();
            }
            return self.frame_state(false, false, true);
        }
        let mut landed = false;
        let mut moved = false;
        let displays = desktop_displays(cx);
        if let Some(sim) = &mut self.simulation {
            let before = sim.position();
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
            if sim.physics_dynamic() {
                let outcome = sim.step_physics_capped(elapsed_ms, &self.physics_platforms);
                self.diagnostics.simulation_steps = sim.steps;
                moved = outcome.moved || sim.position() != before || self.pending_native_move;
                landed = outcome.landed;
                if landed {
                    if let Some(contact) = outcome.contact.as_ref() {
                        if contact.kind == WalkableSurfaceKind::WindowTop {
                            if let Some(window_id) =
                                window_id_for_contact(contact, &self.physics_windows)
                            {
                                self.attachment = Some(WindowAttachment {
                                    id: window_id,
                                    top_edge_offset: sim.simulation.position.x
                                        - self
                                            .physics_windows
                                            .iter()
                                            .find(|window| window.id == window_id)
                                            .map(|window| f32::from(window.bounds.origin.x))
                                            .unwrap_or(0.0),
                                    perched: true,
                                });
                                self.attachment_generation =
                                    self.attachment_generation.wrapping_add(1);
                                self.attachment_timer_scheduled = false;
                            }
                        }
                    }
                    self.clear_physics_snapshot();
                }
            } else {
                let outcome = sim.step_capped(elapsed_ms);
                self.diagnostics.simulation_steps = sim.steps;
                moved = outcome.moved || sim.position() != before || self.pending_native_move;
                landed = outcome.landed;
                if landed && self.attachment.is_some() {
                    self.mark_attachment_perched();
                }
            }
            self.pending_native_move = false;
            if moved {
                self.diagnostics.native_moves += 1;
            }
        }
        self.frame_state(moved, landed, reduced_motion)
    }

    fn frame_state(&self, moved: bool, landed: bool, reduced_motion: bool) -> CharacterFrameState {
        CharacterFrameState {
            position: self.simulation.as_ref().map(|sim| sim.position()),
            moved,
            moving: self
                .simulation
                .as_ref()
                .is_some_and(|sim| sim.moving(reduced_motion)),
            runtime_visual_state: self
                .simulation
                .as_ref()
                .map(runtime_visual_state)
                .unwrap_or(CharacterVisualState::Idle),
            landed,
            reduced_motion,
            attached: self.attachment.is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepOutcome {
    moved: bool,
    landed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttachmentFollowOutcome {
    attached: bool,
    moved: bool,
    restart_dynamic_frames: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseLifecycle {
    RequestPhysicsFrame,
    StartAttachmentFollow,
    ScheduleRoam,
}

impl AttachmentFollowOutcome {
    fn inactive() -> Self {
        Self {
            attached: false,
            moved: false,
            restart_dynamic_frames: false,
        }
    }

    fn dynamic_restart() -> Self {
        Self {
            attached: false,
            moved: false,
            restart_dynamic_frames: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CharacterFrameState {
    position: Option<Point<Pixels>>,
    moved: bool,
    moving: bool,
    runtime_visual_state: CharacterVisualState,
    landed: bool,
    reduced_motion: bool,
    attached: bool,
}

pub struct CharacterOverlayView {
    runtime: Entity<DesktopCharacterRuntime>,
    character: CompanionCharacterId,
    main_window: AnyWindowHandle,
    frame_scheduled: bool,
    drag: Option<CharacterDrag>,
    visual_state: CharacterVisualState,
    pose_started_at: Instant,
    facing: f32,
    last_origin: Option<Point<Pixels>>,
    reduced_motion: bool,
    reaction_generation: u64,
    idle_timer_scheduled: bool,
    idle_generation: u64,
}

#[derive(Debug, Clone, Copy)]
struct CharacterDrag {
    grab_offset: Point<Pixels>,
    start_origin: Point<Pixels>,
    last_origin: Point<Pixels>,
    last_sample_at: Instant,
    release_velocity: Point2,
    moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharacterVisualState {
    Idle,
    IdleFlourish,
    Walking,
    Flying,
    Dragging,
    Snapping,
    Reacting,
    Landing,
}

impl CharacterOverlayView {
    fn new(
        main_window: AnyWindowHandle,
        character: CompanionCharacterId,
        reduced_motion: bool,
        runtime: Entity<DesktopCharacterRuntime>,
    ) -> Self {
        Self {
            runtime,
            character,
            main_window,
            frame_scheduled: false,
            drag: None,
            visual_state: CharacterVisualState::Idle,
            pose_started_at: Instant::now(),
            facing: 1.0,
            last_origin: None,
            reduced_motion,
            reaction_generation: 0,
            idle_timer_scheduled: false,
            idle_generation: 0,
        }
    }

    fn set_visual_state(&mut self, state: CharacterVisualState) {
        if self.visual_state != state {
            self.visual_state = state;
            self.pose_started_at = Instant::now();
        }
    }

    fn cancel_idle_flourish(&mut self) {
        self.idle_generation = self.idle_generation.wrapping_add(1);
        self.idle_timer_scheduled = false;
    }

    fn schedule_idle_flourish(&mut self, cx: &mut Context<Self>) {
        if self.reduced_motion
            || self.idle_timer_scheduled
            || self.drag.is_some()
            || self.visual_state != CharacterVisualState::Idle
        {
            return;
        }
        self.idle_timer_scheduled = true;
        self.idle_generation = self.idle_generation.wrapping_add(1);
        let generation = self.idle_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(IDLE_FLOURISH_DELAY).await;
            let _ = this.update(cx, |view, cx| {
                if view.idle_generation != generation {
                    return;
                }
                view.idle_timer_scheduled = false;
                if view.reduced_motion
                    || view.drag.is_some()
                    || view.visual_state != CharacterVisualState::Idle
                {
                    return;
                }
                view.set_visual_state(CharacterVisualState::IdleFlourish);
                cx.notify();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(IDLE_FLOURISH_DURATION).await;
                    let _ = this.update(cx, |view, cx| {
                        if view.idle_generation == generation
                            && view.drag.is_none()
                            && view.visual_state == CharacterVisualState::IdleFlourish
                        {
                            view.set_visual_state(CharacterVisualState::Idle);
                            view.schedule_idle_flourish(cx);
                            cx.notify();
                        }
                    });
                })
                .detach();
            });
        })
        .detach();
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
            last_origin: self
                .runtime
                .read(cx)
                .current_position()
                .unwrap_or(window.bounds().origin),
            last_sample_at: Instant::now(),
            release_velocity: Point2::new(0.0, 0.0),
            moved: false,
        });
        self.cancel_idle_flourish();
        self.set_visual_state(CharacterVisualState::Dragging);
        self.reaction_generation = self.reaction_generation.wrapping_add(1);
        self.runtime.update(cx, |runtime, _cx| {
            runtime.begin_user_drag();
        });
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
        drag.moved |= drag_crossed_threshold(
            drag.start_origin,
            origin,
            desktop_pointer_scale(window.scale_factor()),
        );
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(drag.last_sample_at);
        if elapsed > Duration::ZERO && elapsed <= DRAG_VELOCITY_SAMPLE_LIMIT {
            drag.release_velocity = bounded_release_velocity(drag.last_origin, origin, elapsed);
        }
        drag.last_origin = origin;
        drag.last_sample_at = now;
        self.drag = Some(drag);
        if !drag.moved {
            cx.stop_propagation();
            return;
        }
        let outcome = self
            .runtime
            .update(cx, |runtime, cx| runtime.drag_to(origin, cx));
        if let Some(outcome) = outcome {
            self.set_visual_state(if outcome.snapped {
                CharacterVisualState::Snapping
            } else {
                CharacterVisualState::Dragging
            });
            if outcome.moved {
                if let Err(error) = window.set_window_origin(outcome.position) {
                    tracing::warn!(error = %error, "desktop character drag movement failed");
                    self.runtime.update(cx, |runtime, _cx| {
                        runtime.record_movement_error(error.to_string())
                    });
                }
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
            let release_velocity = release_velocity_at_mouse_up(
                drag.release_velocity,
                drag.last_sample_at,
                Instant::now(),
            );
            let dynamic = self.runtime.update(cx, |runtime, cx| {
                runtime.finish_user_drag(release_velocity, runtime_entity.clone(), cx)
            });
            self.show_landing(window, cx);
            dynamic
        } else {
            self.runtime.update(cx, |runtime, cx| {
                runtime.react_to_click(event.click_count, runtime_entity.clone(), cx)
            })
        };
        if !drag.moved {
            self.show_reaction(window, cx);
        }
        if started_motion || visual_state_needs_frames(self.visual_state, self.reduced_motion) {
            self.schedule_frame(window, cx);
        }
        cx.stop_propagation();
    }

    fn show_reaction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_idle_flourish();
        self.set_visual_state(CharacterVisualState::Reacting);
        self.reaction_generation = self.reaction_generation.wrapping_add(1);
        let generation = self.reaction_generation;
        cx.notify();
        self.schedule_frame(window, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REACTION_DURATION).await;
            let _ = this.update(cx, |view, cx| {
                if view.reaction_generation == generation && view.drag.is_none() {
                    view.set_visual_state(CharacterVisualState::Idle);
                    view.schedule_idle_flourish(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn show_landing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reduced_motion {
            self.set_visual_state(CharacterVisualState::Idle);
            cx.notify();
            return;
        }
        self.cancel_idle_flourish();
        self.set_visual_state(CharacterVisualState::Landing);
        self.reaction_generation = self.reaction_generation.wrapping_add(1);
        let generation = self.reaction_generation;
        cx.notify();
        self.schedule_frame(window, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LANDING_DURATION).await;
            let _ = this.update(cx, |view, cx| {
                if view.reaction_generation == generation && view.drag.is_none() {
                    view.set_visual_state(CharacterVisualState::Idle);
                    view.schedule_idle_flourish(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn update_facing(&mut self, position: Point<Pixels>) {
        if let Some(last) = self.last_origin {
            let dx = f32::from(position.x) - f32::from(last.x);
            if dx.abs() > 0.5 {
                self.facing = dx.signum();
            }
        }
        self.last_origin = Some(position);
    }

    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_scheduled {
            return;
        }
        self.frame_scheduled = true;
        cx.on_next_frame(window, |view, window, cx| {
            view.frame_scheduled = false;
            let state = view.runtime.update(cx, |runtime, cx| runtime.on_frame(cx));
            view.reduced_motion = state.reduced_motion;
            if let Some(position) = state.position {
                let must_apply_origin =
                    should_apply_native_origin(state.moved, view.last_origin.is_some());
                if must_apply_origin {
                    view.update_facing(position);
                    if let Err(error) = window.set_window_origin(position) {
                        tracing::warn!(error = %error, "desktop character native movement failed");
                        view.runtime.update(cx, |runtime, _cx| {
                            runtime.record_movement_error(error.to_string())
                        });
                    }
                }
            }
            if view.drag.is_none()
                && !matches!(view.visual_state, CharacterVisualState::Reacting)
                && !matches!(view.visual_state, CharacterVisualState::Landing)
                && view.visual_state != state.runtime_visual_state
            {
                if state.runtime_visual_state != CharacterVisualState::Idle {
                    view.cancel_idle_flourish();
                }
                view.set_visual_state(state.runtime_visual_state);
            }
            if state.landed && view.drag.is_none() {
                view.show_landing(window, cx);
            }
            if state.attached {
                let runtime_entity = view.runtime.clone();
                view.runtime.update(cx, |runtime, cx| {
                    runtime.schedule_attachment_follow(runtime_entity, cx)
                });
            } else if should_schedule_roam_after_landing(
                state.landed,
                state.attached,
                view.drag.is_some(),
            ) {
                let runtime_entity = view.runtime.clone();
                view.runtime
                    .update(cx, |runtime, cx| runtime.schedule_roam(runtime_entity, cx));
            }
            if should_schedule_next_frame(state.moving, view.visual_state, state.reduced_motion) {
                view.schedule_frame(window, cx);
            } else if view.visual_state == CharacterVisualState::Idle {
                view.schedule_idle_flourish(cx);
            }
            cx.notify();
        });
    }
}

impl Render for CharacterOverlayView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.main_window;
        let phase = procedural_pose_phase(self.pose_started_at.elapsed());
        let pose = character_pose(self.character, self.visual_state, phase, self.facing);
        let mascot_base = div()
            .absolute()
            .inset_0()
            .top(px(pose.float_y))
            .scale_xy(pose.scale_x * self.facing, pose.scale_y)
            .rotate(pose.rotation_deg)
            .transform_origin(0.5, 0.82)
            .opacity(pose.opacity)
            .child(
                img(character_idle_asset(self.character))
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Contain),
            );
        let mascot =
            if !self.reduced_motion && self.visual_state == CharacterVisualState::IdleFlourish {
                mascot_base
                    .with_animation(
                        SharedString::from(format!(
                            "desktop-character-{:?}-idle-flourish-{}",
                            self.character, self.idle_generation
                        )),
                        Animation::new(IDLE_FLOURISH_DURATION).with_easing(ease_in_out),
                        |element, delta| {
                            let wave = (delta * std::f32::consts::TAU).sin();
                            let blink = if delta > 0.78 && delta < 0.84 {
                                0.975
                            } else {
                                1.0
                            };
                            element
                                .top(px(wave * -3.0))
                                .scale_xy(1.0 + wave * 0.012, blink)
                                .rotate(wave * 1.8)
                        },
                    )
                    .into_any_element()
            } else {
                mascot_base.into_any_element()
            };

        let shadow = div()
            .absolute()
            .left(px(26.0))
            .right(px(26.0))
            .bottom(px(9.0))
            .h(px(15.0))
            .rounded_full()
            .bg(gpui::black().opacity(pose.shadow_opacity))
            .scale_xy(pose.shadow_scale, 1.0)
            .transform_origin(0.5, 0.5);

        let sparkle = div()
            .absolute()
            .right(px(20.0 + pose.sparkle_offset))
            .top(px(18.0 - pose.float_y * 0.35))
            .size(px(13.0))
            .rounded_full()
            .bg(gpui::white().opacity(pose.sparkle_opacity))
            .scale(pose.sparkle_scale);

        let character = div()
            .id("desktop-character-interaction-surface")
            .relative()
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
            .child(shadow)
            .child(mascot)
            .child(sparkle);

        div()
            .size_full()
            .bg(gpui::transparent_black())
            .child(character)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CharacterPersonality {
    base_speed: f32,
    return_speed: f32,
    click_speed: f32,
    double_click_speed: f32,
    climb_speed: f32,
    roam_speed_min: f32,
    roam_speed_max: f32,
    vertical_min: f32,
    vertical_max: f32,
    bounce: f32,
    stride: f32,
    tilt: f32,
    airborne: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PlayfulTarget {
    point: Point2,
    action_id: &'static str,
    speed: f32,
    airborne: bool,
}

impl PlayfulTarget {
    fn with_speed(mut self, profile: CharacterPersonality, rng: &mut DeterministicRng) -> Self {
        self.speed = rng_f32(rng, profile.roam_speed_min, profile.roam_speed_max);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CharacterPose {
    float_y: f32,
    scale_x: f32,
    scale_y: f32,
    rotation_deg: f32,
    opacity: f32,
    shadow_scale: f32,
    shadow_opacity: f32,
    sparkle_opacity: f32,
    sparkle_scale: f32,
    sparkle_offset: f32,
}

fn character_personality(character: CompanionCharacterId) -> CharacterPersonality {
    match character {
        CompanionCharacterId::None | CompanionCharacterId::Clippy => CharacterPersonality {
            base_speed: 132.0,
            return_speed: 158.0,
            click_speed: 185.0,
            double_click_speed: 270.0,
            climb_speed: 118.0,
            roam_speed_min: 112.0,
            roam_speed_max: 168.0,
            vertical_min: 0.72,
            vertical_max: 0.95,
            bounce: 8.0,
            stride: 1.0,
            tilt: 5.5,
            airborne: false,
        },
        CompanionCharacterId::Shelly => CharacterPersonality {
            base_speed: 104.0,
            return_speed: 132.0,
            click_speed: 150.0,
            double_click_speed: 218.0,
            climb_speed: 94.0,
            roam_speed_min: 86.0,
            roam_speed_max: 126.0,
            vertical_min: 0.76,
            vertical_max: 0.96,
            bounce: 5.5,
            stride: 0.72,
            tilt: 3.5,
            airborne: false,
        },
        CompanionCharacterId::Spark => CharacterPersonality {
            base_speed: 184.0,
            return_speed: 220.0,
            click_speed: 245.0,
            double_click_speed: 340.0,
            climb_speed: 150.0,
            roam_speed_min: 172.0,
            roam_speed_max: 252.0,
            vertical_min: 0.18,
            vertical_max: 0.68,
            bounce: 14.0,
            stride: 1.48,
            tilt: 9.0,
            airborne: true,
        },
        CompanionCharacterId::Byte => CharacterPersonality {
            base_speed: 148.0,
            return_speed: 170.0,
            click_speed: 192.0,
            double_click_speed: 275.0,
            climb_speed: 142.0,
            roam_speed_min: 122.0,
            roam_speed_max: 188.0,
            vertical_min: 0.48,
            vertical_max: 0.88,
            bounce: 6.5,
            stride: 1.22,
            tilt: 4.5,
            airborne: false,
        },
        CompanionCharacterId::Orbit => CharacterPersonality {
            base_speed: 124.0,
            return_speed: 154.0,
            click_speed: 178.0,
            double_click_speed: 250.0,
            climb_speed: 118.0,
            roam_speed_min: 112.0,
            roam_speed_max: 158.0,
            vertical_min: 0.22,
            vertical_max: 0.62,
            bounce: 11.0,
            stride: 0.86,
            tilt: 7.0,
            airborne: true,
        },
        CompanionCharacterId::Nox => CharacterPersonality {
            base_speed: 116.0,
            return_speed: 146.0,
            click_speed: 164.0,
            double_click_speed: 232.0,
            climb_speed: 112.0,
            roam_speed_min: 96.0,
            roam_speed_max: 148.0,
            vertical_min: 0.54,
            vertical_max: 0.92,
            bounce: 4.0,
            stride: 0.92,
            tilt: 6.0,
            airborne: false,
        },
    }
}

fn character_seed(character: CompanionCharacterId) -> u64 {
    match character {
        CompanionCharacterId::None | CompanionCharacterId::Clippy => 0xC11F_F11E,
        CompanionCharacterId::Shelly => 0x5E11_0001,
        CompanionCharacterId::Spark => 0x5A11_5A11,
        CompanionCharacterId::Byte => 0xB17E_B17E,
        CompanionCharacterId::Orbit => 0x0B17_0001,
        CompanionCharacterId::Nox => 0xA10C_0001,
    }
}

fn rng_f32(rng: &mut DeterministicRng, min: f32, max: f32) -> f32 {
    let unit = rng.next_u32() as f32 / u32::MAX as f32;
    min + (max - min) * unit
}

fn playful_targets(
    character: CompanionCharacterId,
    bounds: Bounds<Pixels>,
    extent: f32,
    position: Point2,
    rng: &mut DeterministicRng,
) -> Vec<PlayfulTarget> {
    let profile = character_personality(character);
    let min_x = f32::from(bounds.origin.x) + STATIC_MARGIN;
    let max_x = (f32::from(bounds.right()) - extent - STATIC_MARGIN).max(min_x);
    let min_y = f32::from(bounds.origin.y) + STATIC_MARGIN;
    let max_y = (f32::from(bounds.bottom()) - extent - STATIC_MARGIN).max(min_y);
    let width = (max_x - min_x).max(1.0);
    let height = (max_y - min_y).max(1.0);
    let left = min_x + width * rng_f32(rng, 0.03, 0.18);
    let right = min_x + width * rng_f32(rng, 0.82, 0.97);
    let center = min_x + width * rng_f32(rng, 0.40, 0.60);
    let air_y = min_y + height * rng_f32(rng, profile.vertical_min, profile.vertical_max);
    let floor_y = max_y;
    let far_x = if position.x < (min_x + max_x) * 0.5 {
        right
    } else {
        left
    };
    let near_x = (position.x + rng_f32(rng, -width * 0.28, width * 0.28)).clamp(min_x, max_x);

    vec![
        PlayfulTarget {
            point: Point2::new(far_x, floor_y),
            action_id: "roam-edge-scamp",
            speed: profile.base_speed,
            airborne: false,
        },
        PlayfulTarget {
            point: Point2::new(center, air_y),
            action_id: "roam-float-arc",
            speed: profile.base_speed,
            airborne: true,
        },
        PlayfulTarget {
            point: Point2::new(near_x, (air_y * 0.45 + floor_y * 0.55).clamp(min_y, max_y)),
            action_id: "roam-curious-peek",
            speed: profile.base_speed,
            airborne: profile.airborne,
        },
        PlayfulTarget {
            point: Point2::new(if far_x == left { right } else { left }, floor_y),
            action_id: "roam-corner-loop",
            speed: profile.base_speed,
            airborne: false,
        },
    ]
}

fn character_pose(
    character: CompanionCharacterId,
    state: CharacterVisualState,
    phase: f32,
    facing: f32,
) -> CharacterPose {
    let profile = character_personality(character);
    let wave = phase.sin();
    let quick = (phase * profile.stride * 1.8).sin();
    let base = CharacterPose {
        float_y: 0.0,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_deg: 0.0,
        opacity: 1.0,
        shadow_scale: 0.72,
        shadow_opacity: 0.22,
        sparkle_opacity: 0.0,
        sparkle_scale: 0.5,
        sparkle_offset: 0.0,
    };
    match state {
        CharacterVisualState::Idle => CharacterPose {
            shadow_scale: 0.70,
            shadow_opacity: 0.18,
            ..base
        },
        CharacterVisualState::IdleFlourish => CharacterPose {
            shadow_scale: 0.70,
            shadow_opacity: 0.18,
            ..base
        },
        CharacterVisualState::Walking => CharacterPose {
            float_y: -quick.abs() * profile.bounce,
            scale_x: 1.0 + quick.abs() * 0.035,
            scale_y: 1.0 - quick.abs() * 0.03,
            rotation_deg: facing * quick * profile.tilt,
            shadow_scale: 0.62 + quick.abs() * 0.18,
            shadow_opacity: 0.24 - quick.abs() * 0.06,
            sparkle_opacity: 0.08 + quick.abs() * 0.12,
            sparkle_scale: 0.45 + quick.abs() * 0.25,
            sparkle_offset: quick * 5.0,
            ..base
        },
        CharacterVisualState::Flying => CharacterPose {
            float_y: -profile.bounce - wave * profile.bounce * 0.42,
            scale_x: 1.0 + wave * 0.025,
            scale_y: 1.0 - wave * 0.018,
            rotation_deg: facing * (profile.tilt + wave * profile.tilt * 0.65),
            shadow_scale: 0.48 + wave.abs() * 0.10,
            shadow_opacity: 0.13,
            sparkle_opacity: 0.24 + wave.abs() * 0.22,
            sparkle_scale: 0.65 + wave.abs() * 0.30,
            sparkle_offset: wave * 9.0,
            ..base
        },
        CharacterVisualState::Dragging => CharacterPose {
            float_y: 5.0 + wave * 1.5,
            scale_x: 1.08,
            scale_y: 0.91,
            rotation_deg: facing * (6.0 + wave * 2.0),
            shadow_scale: 0.82,
            shadow_opacity: 0.28,
            sparkle_opacity: 0.18,
            sparkle_scale: 0.65,
            ..base
        },
        CharacterVisualState::Snapping => CharacterPose {
            float_y: -3.0,
            scale_x: 1.04,
            scale_y: 0.96,
            rotation_deg: 0.0,
            shadow_scale: 0.86,
            shadow_opacity: 0.32,
            sparkle_opacity: 0.55,
            sparkle_scale: 0.82,
            sparkle_offset: 0.0,
            ..base
        },
        CharacterVisualState::Reacting => CharacterPose {
            float_y: -12.0 - quick.abs() * 10.0,
            scale_x: 1.08 + quick.abs() * 0.05,
            scale_y: 0.96 + quick.abs() * 0.05,
            rotation_deg: facing * quick * profile.tilt * 1.35,
            shadow_scale: 0.56 + quick.abs() * 0.12,
            shadow_opacity: 0.16,
            sparkle_opacity: 0.50 + quick.abs() * 0.35,
            sparkle_scale: 0.82 + quick.abs() * 0.28,
            sparkle_offset: quick * 12.0,
            ..base
        },
        CharacterVisualState::Landing => CharacterPose {
            float_y: 3.5 - quick.abs() * 2.0,
            scale_x: 1.10 - quick.abs() * 0.04,
            scale_y: 0.88 + quick.abs() * 0.06,
            rotation_deg: facing * wave * profile.tilt * 0.42,
            shadow_scale: 0.84,
            shadow_opacity: 0.30,
            sparkle_opacity: 0.18 + quick.abs() * 0.18,
            sparkle_scale: 0.70,
            sparkle_offset: quick * 6.0,
            ..base
        },
    }
}

fn runtime_visual_state(sim: &RuntimeSimulation) -> CharacterVisualState {
    match sim.simulation.state {
        CharacterSimulationState::Walking
        | CharacterSimulationState::Climbing
        | CharacterSimulationState::ReturningToDock => CharacterVisualState::Walking,
        CharacterSimulationState::Jumping | CharacterSimulationState::Flying => {
            CharacterVisualState::Flying
        }
        CharacterSimulationState::Landing => CharacterVisualState::Landing,
        _ => CharacterVisualState::Idle,
    }
}

fn visual_state_needs_frames(state: CharacterVisualState, reduced_motion: bool) -> bool {
    !reduced_motion
        && matches!(
            state,
            CharacterVisualState::Walking
                | CharacterVisualState::Flying
                | CharacterVisualState::Reacting
                | CharacterVisualState::Landing
        )
}

fn should_schedule_next_frame(
    moving: bool,
    visual_state: CharacterVisualState,
    reduced_motion: bool,
) -> bool {
    !reduced_motion && (moving || visual_state_needs_frames(visual_state, reduced_motion))
}

fn should_apply_native_origin(moved: bool, has_last_origin: bool) -> bool {
    moved || !has_last_origin
}

fn should_schedule_roam_after_landing(landed: bool, attached: bool, dragging: bool) -> bool {
    landed && !attached && !dragging
}

fn procedural_pose_phase(elapsed: Duration) -> f32 {
    elapsed.as_secs_f32() * std::f32::consts::TAU
}

fn reduced_motion_for_config(config: &ClippyConfig) -> bool {
    matches!(
        config.appearance.motion,
        CompanionMotionPreference::Reduced | CompanionMotionPreference::Off
    ) || config.appearance.desktop.movement == DesktopCompanionMovement::Still
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

fn drag_threshold_squared(pointer_scale: f32) -> f32 {
    let threshold = DRAG_THRESHOLD * pointer_scale.max(1.0);
    threshold * threshold
}

fn drag_crossed_threshold(
    start_origin: Point<Pixels>,
    current_origin: Point<Pixels>,
    pointer_scale: f32,
) -> bool {
    let dx = f32::from(current_origin.x) - f32::from(start_origin.x);
    let dy = f32::from(current_origin.y) - f32::from(start_origin.y);
    dx * dx + dy * dy >= drag_threshold_squared(pointer_scale)
}

fn bounded_release_velocity(from: Point<Pixels>, to: Point<Pixels>, elapsed: Duration) -> Point2 {
    let seconds = elapsed.as_secs_f32().max(0.001);
    let config = PhysicsConfig::default();
    Point2::new(
        ((f32::from(to.x) - f32::from(from.x)) / seconds)
            .clamp(-config.max_horizontal_speed, config.max_horizontal_speed),
        ((f32::from(to.y) - f32::from(from.y)) / seconds)
            .clamp(-config.terminal_velocity, config.terminal_velocity),
    )
}

fn release_velocity_at_mouse_up(
    sampled_velocity: Point2,
    last_sample_at: Instant,
    mouse_up_at: Instant,
) -> Point2 {
    if mouse_up_at.saturating_duration_since(last_sample_at) > DRAG_VELOCITY_SAMPLE_LIMIT {
        Point2::new(0.0, 0.0)
    } else {
        sampled_velocity
    }
}

fn drag_snapshot_refresh_due(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= interval)
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

fn scale_coordinate_distance(logical_distance: f32, scale_factor: f32, physical: bool) -> f32 {
    if physical {
        logical_distance * scale_factor
    } else {
        logical_distance
    }
}

fn desktop_coordinate_distance(logical_distance: f32, scale_factor: f32) -> f32 {
    scale_coordinate_distance(logical_distance, scale_factor, cfg!(target_os = "windows"))
}

fn snap_coordinate_metrics(
    logical_extent: f32,
    logical_vertical_gap: f32,
    logical_horizontal_overlap: f32,
    scale_factor: f32,
    physical: bool,
) -> (f32, f32, f32) {
    (
        scale_coordinate_distance(logical_extent, scale_factor, physical),
        scale_coordinate_distance(logical_vertical_gap, scale_factor, physical),
        scale_coordinate_distance(logical_horizontal_overlap, scale_factor, physical),
    )
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

fn display_for_window(
    displays: &[DesktopDisplay],
    window: Bounds<Pixels>,
) -> Option<DesktopDisplay> {
    displays
        .iter()
        .copied()
        .find(|display| display.work_area.contains(&window.center()))
        .or_else(|| display_for_overlay_origin(displays, window.origin, 0.0))
}

fn choose_external_window(
    windows: &[ExternalWindow],
    displays: &[DesktopDisplay],
    position: Point2,
    restricted_display: Option<DesktopDisplay>,
) -> Option<ExternalWindow> {
    windows
        .iter()
        .filter(|window| {
            valid_physics_window(window.bounds, displays)
                && restricted_display.is_none_or(|restricted| {
                    display_for_window(displays, window.bounds)
                        .is_some_and(|candidate| same_display(candidate, restricted))
                })
        })
        .cloned()
        .min_by(|a, b| {
            let rank = |window: &ExternalWindow| {
                let center = window.bounds.center();
                (
                    (f32::from(window.bounds.origin.y) - position.y).abs(),
                    (f32::from(center.x) - position.x).abs(),
                    window.id.raw(),
                )
            };
            let a = rank(a);
            let b = rank(b);
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        })
}

fn same_display(a: DesktopDisplay, b: DesktopDisplay) -> bool {
    match (a.id, b.id) {
        (Some(a), Some(b)) => a == b,
        _ => a.work_area == b.work_area,
    }
}

#[derive(Debug, Clone)]
struct SnapCandidate {
    window: ExternalWindow,
    display: DesktopDisplay,
    extent: f32,
    min_horizontal_overlap: f32,
    generation: u64,
}

fn choose_snap_window(
    windows: &[ExternalWindow],
    displays: &[DesktopDisplay],
    position: Point2,
    logical_extent: f32,
) -> Option<SnapCandidate> {
    choose_snap_window_with_limits(
        windows,
        displays,
        position,
        logical_extent,
        SNAP_VERTICAL_GAP,
        SNAP_MIN_HORIZONTAL_OVERLAP,
    )
}

fn choose_magnetic_snap_window(
    windows: &[ExternalWindow],
    displays: &[DesktopDisplay],
    position: Point2,
    logical_extent: f32,
    preferred: Option<ExternalWindowId>,
) -> Option<SnapCandidate> {
    preferred
        .and_then(|id| {
            choose_snap_window_for_id(
                windows,
                displays,
                position,
                logical_extent,
                id,
                SNAP_HYSTERESIS_VERTICAL_GAP,
                SNAP_HYSTERESIS_MIN_HORIZONTAL_OVERLAP,
            )
        })
        .or_else(|| {
            choose_snap_window_with_limits(
                windows,
                displays,
                position,
                logical_extent,
                SNAP_PREVIEW_VERTICAL_GAP,
                SNAP_MIN_HORIZONTAL_OVERLAP,
            )
        })
}

fn choose_snap_window_for_id(
    windows: &[ExternalWindow],
    displays: &[DesktopDisplay],
    position: Point2,
    logical_extent: f32,
    id: ExternalWindowId,
    max_vertical_gap: f32,
    min_horizontal_overlap: f32,
) -> Option<SnapCandidate> {
    windows
        .iter()
        .find(|window| window.id == id)
        .and_then(|window| {
            snap_candidate(
                window,
                displays,
                position,
                logical_extent,
                max_vertical_gap,
                min_horizontal_overlap,
            )
            .map(|(_, _, candidate)| candidate)
        })
}

fn choose_snap_window_with_limits(
    windows: &[ExternalWindow],
    displays: &[DesktopDisplay],
    position: Point2,
    logical_extent: f32,
    max_vertical_gap: f32,
    min_horizontal_overlap: f32,
) -> Option<SnapCandidate> {
    windows
        .iter()
        .filter_map(|window| {
            snap_candidate(
                window,
                displays,
                position,
                logical_extent,
                max_vertical_gap,
                min_horizontal_overlap,
            )
        })
        .min_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.generation.cmp(&b.2.generation))
        })
        .map(|(_, _, candidate)| candidate)
}

fn snap_candidate(
    window: &ExternalWindow,
    displays: &[DesktopDisplay],
    position: Point2,
    logical_extent: f32,
    max_vertical_gap: f32,
    min_horizontal_overlap: f32,
) -> Option<(f32, f32, SnapCandidate)> {
    let display = display_for_window(displays, window.bounds)?;
    if !valid_physics_window(window.bounds, displays) {
        return None;
    }
    let (extent, max_vertical_gap, eligibility_overlap) = snap_coordinate_metrics(
        logical_extent,
        max_vertical_gap,
        min_horizontal_overlap,
        display.scale_factor,
        cfg!(target_os = "windows"),
    );
    let eligibility_overlap = eligibility_overlap
        .min(extent)
        .min(f32::from(window.bounds.size.width));
    let perch_overlap =
        desktop_coordinate_distance(SNAP_MIN_HORIZONTAL_OVERLAP, display.scale_factor)
            .min(extent)
            .min(f32::from(window.bounds.size.width));
    let mascot_left = position.x;
    let mascot_right = position.x + extent;
    let mascot_bottom = position.y + extent;
    let top = f32::from(window.bounds.origin.y);
    let gap = (mascot_bottom - top).abs();
    if gap > max_vertical_gap {
        return None;
    }
    let left = f32::from(window.bounds.origin.x);
    let right = f32::from(window.bounds.right());
    let overlap = mascot_right.min(right) - mascot_left.max(left);
    if overlap < eligibility_overlap {
        return None;
    }
    let mascot_center = position.x + extent * 0.5;
    let horizontal_distance = if mascot_center < left {
        left - mascot_center
    } else if mascot_center > right {
        mascot_center - right
    } else {
        0.0
    };
    Some((
        gap,
        horizontal_distance,
        SnapCandidate {
            window: window.clone(),
            display,
            extent,
            min_horizontal_overlap: perch_overlap,
            generation: window.id.raw(),
        },
    ))
}

fn magnetic_snap_position(candidate: &SnapCandidate, desired: Point2) -> Option<Point2> {
    perch_origin(
        candidate.window.bounds,
        candidate.display.work_area,
        candidate.extent,
        desired.x,
        candidate.min_horizontal_overlap,
    )
}

fn valid_physics_window(window: Bounds<Pixels>, displays: &[DesktopDisplay]) -> bool {
    let Some(display) = display_for_window(displays, window) else {
        return false;
    };
    let min_width = desktop_coordinate_distance(80.0, display.scale_factor);
    let min_height = desktop_coordinate_distance(40.0, display.scale_factor);
    f32::from(window.size.width) >= min_width
        && f32::from(window.size.height) >= min_height
        && !maximized_like(window, display.work_area, display.scale_factor)
}

fn maximized_like(window: Bounds<Pixels>, work_area: Bounds<Pixels>, scale_factor: f32) -> bool {
    let work_width = f32::from(work_area.size.width).max(1.0);
    let work_height = f32::from(work_area.size.height).max(1.0);
    let width_coverage = f32::from(window.size.width) / work_width;
    let height_coverage = f32::from(window.size.height) / work_height;
    let edge_tolerance = desktop_coordinate_distance(MAXIMIZED_EDGE_INSET_TOLERANCE, scale_factor);
    width_coverage >= MAXIMIZED_COVERAGE_RATIO
        && height_coverage >= MAXIMIZED_COVERAGE_RATIO
        && (f32::from(window.origin.x) - f32::from(work_area.origin.x)).abs() <= edge_tolerance
        && (f32::from(window.origin.y) - f32::from(work_area.origin.y)).abs() <= edge_tolerance
        && (f32::from(window.right()) - f32::from(work_area.right())).abs() <= edge_tolerance
        && (f32::from(window.bottom()) - f32::from(work_area.bottom())).abs() <= edge_tolerance
}

fn window_top_surface(window: &ExternalWindow, generation: u64) -> WalkableSurface {
    WalkableSurface {
        id: SurfaceId(format!("external:{:?}:top", window.id)),
        kind: WalkableSurfaceKind::WindowTop,
        segment: LineSegment {
            start: Point2::new(
                f32::from(window.bounds.origin.x),
                f32::from(window.bounds.origin.y),
            ),
            end: Point2::new(
                f32::from(window.bounds.right()),
                f32::from(window.bounds.origin.y),
            ),
        },
        normal: Vector2 { x: 0.0, y: -1.0 },
        source_generation: generation,
    }
}

fn physics_platforms(
    windows: &[ExternalWindow],
    displays: &[DesktopDisplay],
) -> Vec<WalkableSurface> {
    windows
        .iter()
        .filter(|window| valid_physics_window(window.bounds, displays))
        .map(|window| window_top_surface(window, window.id.raw()))
        .collect()
}

fn window_id_for_contact(
    contact: &SurfaceContact,
    windows: &[ExternalWindow],
) -> Option<ExternalWindowId> {
    windows
        .iter()
        .find(|window| contact.id == window_top_surface(window, 0).id)
        .map(|window| window.id)
}

fn external_window_id(window: &ExternalWindow) -> ExternalWindowId {
    window.id
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
    min_horizontal_overlap: f32,
) -> Option<Point2> {
    let usable_width = f32::from(window.size.width).min(f32::from(work_area.size.width));
    if usable_width < 80.0 || f32::from(window.size.height) < 40.0 {
        return None;
    }
    perch_origin(
        window,
        work_area,
        extent,
        f32::from(window.center().x) - extent * 0.5,
        min_horizontal_overlap,
    )
}

fn attachment_target(
    window: Bounds<Pixels>,
    work_area: Bounds<Pixels>,
    extent: f32,
    preferred_x: f32,
    min_horizontal_overlap: f32,
) -> Option<Point2> {
    if f32::from(window.size.width) < 80.0 || f32::from(window.size.height) < 40.0 {
        return None;
    }
    perch_origin(
        window,
        work_area,
        extent,
        preferred_x,
        min_horizontal_overlap,
    )
}

fn perch_origin(
    window: Bounds<Pixels>,
    work_area: Bounds<Pixels>,
    extent: f32,
    preferred_x: f32,
    min_horizontal_overlap: f32,
) -> Option<Point2> {
    if f32::from(window.size.width) <= 0.0 || f32::from(window.size.height) <= 0.0 {
        return None;
    }
    let min_x = f32::from(work_area.origin.x);
    let max_x = (f32::from(work_area.right()) - extent).max(min_x);
    let min_y = f32::from(work_area.origin.y);
    let max_y = (f32::from(work_area.bottom()) - extent).max(min_y);
    let window_left = f32::from(window.origin.x);
    let window_right = f32::from(window.right());
    let overlap = min_horizontal_overlap
        .min(extent)
        .min(f32::from(window.size.width));
    let window_min_x = window_left - extent + overlap;
    let window_max_x = window_right - overlap;
    let x = preferred_x
        .clamp(
            window_min_x.min(window_max_x),
            window_min_x.max(window_max_x),
        )
        .clamp(min_x, max_x);
    let y = (f32::from(window.origin.y) - extent).clamp(min_y, max_y);
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

fn raw_rect(bounds: Bounds<Pixels>) -> Rect {
    Rect::new(
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width).max(1.0),
        f32::from(bounds.size.height).max(1.0),
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

#[cfg(test)]
fn character_visual_asset(
    character: CompanionCharacterId,
    _state: CharacterVisualState,
) -> &'static str {
    character_idle_asset(character)
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

    fn runtime_with_sim() -> DesktopCharacterRuntime {
        let mut runtime = DesktopCharacterRuntime::new(config(true));
        runtime.simulation = Some(RuntimeSimulation::new(
            display(),
            character_window_size(CompanionScale::Medium),
            CompanionCharacterId::Clippy,
        ));
        runtime
    }

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: gpui::point(px(x), px(y)),
            size: gpui::size(px(width), px(height)),
        }
    }

    fn external_window(id: u64, bounds: Bounds<Pixels>) -> ExternalWindow {
        ExternalWindow {
            id: ExternalWindowId::from_raw(id),
            bounds,
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
        let mut sim = RuntimeSimulation::new(
            display(),
            character_window_size(CompanionScale::Medium),
            CompanionCharacterId::Clippy,
        );
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
        let mut sim = RuntimeSimulation::new(
            display(),
            character_window_size(CompanionScale::Medium),
            CompanionCharacterId::Clippy,
        );
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
            "characters/orbit/idle.png"
        );
        assert_eq!(
            character_visual_asset(CompanionCharacterId::Nox, CharacterVisualState::Reacting),
            "characters/nox/idle.png"
        );
    }

    // SDTEST-1494
    #[test]
    fn procedural_pose_values_stay_inside_safe_visual_bounds() {
        let characters = [
            CompanionCharacterId::Clippy,
            CompanionCharacterId::Shelly,
            CompanionCharacterId::Spark,
            CompanionCharacterId::Byte,
            CompanionCharacterId::Orbit,
            CompanionCharacterId::Nox,
        ];
        let states = [
            CharacterVisualState::Idle,
            CharacterVisualState::IdleFlourish,
            CharacterVisualState::Walking,
            CharacterVisualState::Flying,
            CharacterVisualState::Dragging,
            CharacterVisualState::Snapping,
            CharacterVisualState::Reacting,
            CharacterVisualState::Landing,
        ];

        for character in characters {
            for state in states {
                for phase in [0.0, 0.6, 1.4, 2.2, 3.1, 4.7] {
                    let pose = character_pose(character, state, phase, 1.0);
                    assert!(pose.float_y >= -26.0 && pose.float_y <= 7.0, "{pose:?}");
                    assert!(pose.scale_x >= 0.92 && pose.scale_x <= 1.15, "{pose:?}");
                    assert!(pose.scale_y >= 0.86 && pose.scale_y <= 1.08, "{pose:?}");
                    assert!(pose.rotation_deg.abs() <= 22.0, "{pose:?}");
                    assert!(pose.opacity >= 0.0 && pose.opacity <= 1.0, "{pose:?}");
                    assert!(
                        pose.sparkle_opacity >= 0.0 && pose.sparkle_opacity <= 0.9,
                        "{pose:?}"
                    );
                }
            }
        }
    }

    // SDTEST-1495
    #[test]
    fn character_personalities_are_visibly_and_kinetically_distinct() {
        let shelly = character_personality(CompanionCharacterId::Shelly);
        let spark = character_personality(CompanionCharacterId::Spark);
        let orbit = character_personality(CompanionCharacterId::Orbit);
        let nox = character_personality(CompanionCharacterId::Nox);

        assert!(spark.airborne);
        assert!(orbit.airborne);
        assert!(!shelly.airborne);
        assert!(spark.roam_speed_min > shelly.roam_speed_max);
        assert!(nox.bounce < spark.bounce);

        let shelly_walk = character_pose(
            CompanionCharacterId::Shelly,
            CharacterVisualState::Walking,
            1.25,
            1.0,
        );
        let spark_flight = character_pose(
            CompanionCharacterId::Spark,
            CharacterVisualState::Flying,
            1.25,
            1.0,
        );
        assert!(spark_flight.float_y < shelly_walk.float_y);
        assert!(spark_flight.rotation_deg.abs() > shelly_walk.rotation_deg.abs());
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
        let target = window_top_target(window, work_area, extent, SNAP_MIN_HORIZONTAL_OVERLAP)
            .expect("eligible external window");
        assert!(to_rect(work_area, extent).contains(target));
        assert!(target.y < f32::from(window.origin.y));
        assert!(window_top_target(
            Bounds {
                size: gpui::size(px(20.0), px(20.0)),
                ..window
            },
            work_area,
            extent,
            SNAP_MIN_HORIZONTAL_OVERLAP,
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
        let mut sim = RuntimeSimulation::new(display(), extent, CompanionCharacterId::Clippy);
        let start = sim.simulation.position;
        sim.simulation
            .set_target(Point2::new(start.x - 100.0, start.y));
        assert!(!sim.step_capped(16).moved);
        assert_eq!(sim.pending_ms, 16);
        assert!(sim.step_capped(17).moved);
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
        let mut sim = RuntimeSimulation::new(display(), extent, CompanionCharacterId::Clippy);
        sim.simulation.position = Point2::new(100.0, 300.0);
        assert!(sim.react_to_click(1, extent));
        assert_eq!(sim.simulation.state, CharacterSimulationState::Jumping);
        assert_eq!(sim.simulation.target, Some(Point2::new(212.0, 300.0)));

        sim.simulation.position = Point2::new(100.0, 300.0);
        assert!(sim.react_to_click(2, extent));
        assert_eq!(sim.simulation.target, Some(Point2::new(640.0, 300.0)));
        assert!(sim.moving(false));
    }

    // SDTEST-1496
    #[test]
    fn playful_targets_are_bounded_varied_and_cooldown_aware() {
        let extent = character_window_size(CompanionScale::Medium);
        let work_area = display().work_area;
        let mut rng = DeterministicRng::seeded(character_seed(CompanionCharacterId::Spark));
        let targets = playful_targets(
            CompanionCharacterId::Spark,
            work_area,
            extent,
            Point2::new(120.0, 400.0),
            &mut rng,
        );
        assert!(targets.len() >= 4);
        let bounds = to_rect(work_area, extent);
        for target in &targets {
            assert!(bounds.contains(target.point), "{target:?}");
        }
        assert!(targets.iter().any(|target| target.airborne));
        assert!(targets
            .windows(2)
            .any(|pair| pair[0].point != pair[1].point));

        let mut sim = RuntimeSimulation::new(display(), extent, CompanionCharacterId::Spark);
        let first = sim.next_playful_target().expect("first target");
        sim.simulation.remember_action(first.action_id);
        let second = sim.next_playful_target().expect("second target");
        assert_ne!(first.action_id, second.action_id);
        assert!(bounds.contains(second.point));
        let profile = character_personality(CompanionCharacterId::Spark);
        assert!(second.speed >= profile.roam_speed_min && second.speed <= profile.roam_speed_max);

        sim.simulation.last_actions.clear();
        sim.simulation.config.cooldown_memory = 3;
        for action in ["roam-edge-scamp", "roam-float-arc", "roam-curious-peek"] {
            sim.simulation.remember_action(action);
        }
        let only_available = sim.next_playful_target().expect("remaining target");
        assert_eq!(only_available.action_id, "roam-corner-loop");
    }

    // SDTEST-1497
    #[test]
    fn frame_scheduling_policy_is_event_driven_and_honors_reduced_motion() {
        assert!(!should_schedule_next_frame(
            false,
            CharacterVisualState::Idle,
            false
        ));
        assert!(!should_schedule_next_frame(
            false,
            CharacterVisualState::IdleFlourish,
            false
        ));
        assert!(should_schedule_next_frame(
            true,
            CharacterVisualState::Walking,
            false
        ));
        assert!(should_schedule_next_frame(
            false,
            CharacterVisualState::Reacting,
            false
        ));
        assert!(should_schedule_next_frame(
            false,
            CharacterVisualState::Landing,
            false
        ));
        assert!(!should_schedule_next_frame(
            true,
            CharacterVisualState::Walking,
            true
        ));

        let mut off = config(true);
        off.appearance.motion = CompanionMotionPreference::Off;
        assert!(reduced_motion_for_config(&off));
        let mut still = config(true);
        still.appearance.desktop.movement = DesktopCompanionMovement::Still;
        assert!(reduced_motion_for_config(&still));
    }

    // SDTEST-1498
    #[test]
    fn dpi_aware_drag_threshold_preserves_clicks_and_native_moves_are_gated() {
        assert_eq!(drag_threshold_squared(1.0), 16.0);
        assert_eq!(drag_threshold_squared(1.5), 36.0);
        assert_eq!(drag_threshold_squared(2.0), 64.0);
        assert!(!drag_crossed_threshold(
            gpui::point(px(100.0), px(100.0)),
            gpui::point(px(105.0), px(100.0)),
            1.5,
        ));
        assert!(drag_crossed_threshold(
            gpui::point(px(100.0), px(100.0)),
            gpui::point(px(106.0), px(100.0)),
            1.5,
        ));

        assert!(should_apply_native_origin(false, false));
        assert!(should_apply_native_origin(true, true));
        assert!(!should_apply_native_origin(false, true));
    }

    // SDTEST-1499
    #[test]
    fn procedural_pose_phase_tracks_wall_clock_time_not_refresh_count() {
        let quarter = procedural_pose_phase(Duration::from_millis(250));
        let half = procedural_pose_phase(Duration::from_millis(500));
        assert!((quarter - std::f32::consts::FRAC_PI_2).abs() < 0.0001);
        assert!((half - std::f32::consts::PI).abs() < 0.0001);
    }

    // SDTEST-1500
    #[test]
    fn attachment_preserves_top_edge_offset_when_window_moves() {
        let mut runtime = runtime_with_sim();
        let initial = bounds(200.0, 260.0, 420.0, 240.0);
        assert!(runtime.begin_attachment(ExternalWindowId::from_raw(42), initial, &[display()]));
        runtime.mark_attachment_perched();
        let moved = bounds(240.0, 300.0, 420.0, 240.0);

        let outcome = runtime.follow_attached_bounds(moved, &[display()]);

        assert_eq!(
            outcome,
            AttachmentFollowOutcome {
                attached: true,
                moved: true,
                restart_dynamic_frames: false,
            }
        );
        let sim = runtime.simulation.as_ref().expect("simulation");
        let attachment = runtime.attachment.expect("attachment");
        assert_eq!(attachment.id, ExternalWindowId::from_raw(42));
        assert_eq!(attachment.top_edge_offset, 130.0);
        assert_eq!(sim.simulation.position, Point2::new(370.0, 140.0));
        assert_eq!(sim.simulation.state, CharacterSimulationState::Perched);
        assert!(runtime.pending_native_move);
    }

    // SDTEST-1501
    #[test]
    fn attachment_clamps_offset_when_window_resizes() {
        let mut runtime = runtime_with_sim();
        let initial = bounds(200.0, 260.0, 420.0, 240.0);
        assert!(runtime.begin_attachment(ExternalWindowId::from_raw(77), initial, &[display()]));
        runtime.mark_attachment_perched();
        let resized = bounds(200.0, 260.0, 100.0, 240.0);

        let outcome = runtime.follow_attached_bounds(resized, &[display()]);

        assert!(outcome.attached);
        let sim = runtime.simulation.as_ref().expect("simulation");
        assert_eq!(sim.simulation.position, Point2::new(264.0, 100.0));
    }

    // SDTEST-1502
    #[test]
    fn attachment_unchanged_window_does_not_request_native_move() {
        let mut runtime = runtime_with_sim();
        let window = bounds(200.0, 260.0, 420.0, 240.0);
        assert!(runtime.begin_attachment(ExternalWindowId::from_raw(7), window, &[display()]));
        runtime.mark_attachment_perched();
        assert!(runtime.follow_attached_bounds(window, &[display()]).moved);
        runtime.pending_native_move = false;

        let outcome = runtime.follow_attached_bounds(window, &[display()]);

        assert_eq!(
            outcome,
            AttachmentFollowOutcome {
                attached: true,
                moved: false,
                restart_dynamic_frames: false,
            }
        );
        assert!(!runtime.pending_native_move);
    }

    // SDTEST-1503
    #[test]
    fn attachment_missing_window_detaches_safely() {
        let mut runtime = runtime_with_sim();
        assert!(runtime.begin_attachment(
            ExternalWindowId::from_raw(9),
            bounds(200.0, 260.0, 420.0, 240.0),
            &[display()]
        ));

        let outcome = runtime.follow_attached_snapshot(
            &[external_window(10, bounds(200.0, 260.0, 420.0, 240.0))],
            &[display()],
        );

        assert_eq!(outcome, AttachmentFollowOutcome::dynamic_restart());
        assert!(runtime.attachment.is_none());
        assert!(!runtime.attachment_timer_scheduled);
    }

    // SDTEST-1504
    #[test]
    fn attachment_cancellation_and_scheduler_policy_bump_generations() {
        let mut runtime = runtime_with_sim();
        assert_eq!(ATTACHMENT_FOLLOW_INTERVAL, Duration::from_millis(100));
        assert!(runtime.begin_attachment(
            ExternalWindowId::from_raw(5),
            bounds(200.0, 260.0, 420.0, 240.0),
            &[display()]
        ));
        let attached_generation = runtime.attachment_generation;

        runtime.begin_user_drag();
        assert!(runtime.attachment.is_none());
        assert!(runtime.attachment_generation > attached_generation);

        assert!(runtime.begin_attachment(
            ExternalWindowId::from_raw(5),
            bounds(200.0, 260.0, 420.0, 240.0),
            &[display()]
        ));
        runtime.attachment_timer_scheduled = true;
        let attached_generation = runtime.attachment_generation;
        runtime.cancel_attachment();
        assert!(runtime.attachment.is_none());
        assert!(!runtime.attachment_timer_scheduled);
        assert!(runtime.attachment_generation > attached_generation);
    }

    // SDTEST-1512
    #[test]
    fn drag_release_snap_candidate_ranking_prefers_vertical_gap_then_horizontal_distance() {
        let windows = [
            external_window(1, bounds(40.0, 302.0, 220.0, 160.0)),
            external_window(2, bounds(200.0, 300.0, 220.0, 160.0)),
        ];
        let chosen = choose_snap_window(&windows, &[display()], Point2::new(210.0, 140.0), 160.0)
            .expect("snap candidate");
        assert_eq!(chosen.window.id, ExternalWindowId::from_raw(2));

        let windows = [
            external_window(1, bounds(40.0, 300.0, 220.0, 160.0)),
            external_window(2, bounds(200.0, 300.0, 220.0, 160.0)),
        ];
        let chosen = choose_snap_window(&windows, &[display()], Point2::new(210.0, 140.0), 160.0)
            .expect("snap candidate");
        assert_eq!(chosen.window.id, ExternalWindowId::from_raw(2));
    }

    // SDTEST-1513
    #[test]
    fn drag_release_rejects_maximized_like_snap_windows() {
        let windows = [external_window(1, bounds(0.0, 0.0, 800.0, 600.0))];
        assert!(
            choose_snap_window(&windows, &[display()], Point2::new(200.0, -155.0), 160.0).is_none()
        );
        assert!(!valid_physics_window(
            bounds(0.0, 0.0, 800.0, 600.0),
            &[display()]
        ));
    }

    // SDTEST-1514
    #[test]
    fn drag_release_does_not_snap_outside_vertical_or_overlap_thresholds() {
        let far_y = [external_window(1, bounds(200.0, 360.0, 220.0, 160.0))];
        assert!(
            choose_snap_window(&far_y, &[display()], Point2::new(210.0, 140.0), 160.0).is_none()
        );
        let far_x = [external_window(1, bounds(500.0, 300.0, 220.0, 160.0))];
        assert!(
            choose_snap_window(&far_x, &[display()], Point2::new(210.0, 140.0), 160.0).is_none()
        );
    }

    // SDTEST-1515
    #[test]
    fn drag_release_velocity_is_bounded_from_logical_desktop_samples() {
        let velocity = bounded_release_velocity(
            gpui::point(px(0.0), px(0.0)),
            gpui::point(px(10_000.0), px(-10_000.0)),
            Duration::from_millis(10),
        );
        let config = PhysicsConfig::default();
        assert_eq!(velocity.x, config.max_horizontal_speed);
        assert_eq!(velocity.y, -config.terminal_velocity);
    }

    // SDTEST-1516
    #[test]
    fn dynamic_fall_lands_on_window_and_maps_attachment() {
        let mut runtime = runtime_with_sim();
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 40.0);
        assert!(runtime.finish_user_drag_snapshot_for_test(
            Point2::new(0.0, 0.0),
            &[],
            &[display()]
        ));
        let window = external_window(44, bounds(200.0, 300.0, 260.0, 180.0));
        let platforms = physics_platforms(std::slice::from_ref(&window), &[display()]);
        let sim = runtime.simulation.as_mut().unwrap();
        let mut landed = false;
        for _ in 0..60 {
            let result = sim.step_physics_capped(33, &platforms);
            if result.landed {
                landed = true;
                assert_eq!(result.contact.unwrap().kind, WalkableSurfaceKind::WindowTop);
                break;
            }
        }
        assert!(landed);
        let contact = runtime
            .simulation
            .as_ref()
            .unwrap()
            .body
            .contact()
            .unwrap()
            .clone();
        assert_eq!(
            window_id_for_contact(&contact, &[window]),
            Some(ExternalWindowId::from_raw(44))
        );
    }

    // SDTEST-1517
    #[test]
    fn dynamic_fall_lands_on_display_floor_without_window() {
        let mut runtime = runtime_with_sim();
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 80.0);
        assert!(runtime.finish_user_drag_snapshot_for_test(
            Point2::new(0.0, 0.0),
            &[],
            &[display()]
        ));
        let sim = runtime.simulation.as_mut().unwrap();
        for _ in 0..90 {
            if sim.step_physics_capped(33, &[]).landed {
                break;
            }
        }
        let contact = sim.body.contact().expect("floor contact");
        assert_eq!(contact.kind, WalkableSurfaceKind::ScreenFloor);
        assert_eq!(sim.body.mode, BodyMode::Sleeping);
    }

    // SDTEST-1518
    #[test]
    fn missing_attached_window_falls_only_when_full_motion_allowed() {
        let mut runtime = runtime_with_sim();
        runtime.attachment = Some(WindowAttachment {
            id: ExternalWindowId::from_raw(9),
            top_edge_offset: 10.0,
            perched: true,
        });
        runtime.detach_missing_attachment();
        assert_eq!(
            runtime.simulation.as_ref().unwrap().body.mode,
            BodyMode::Dynamic
        );

        let mut reduced = runtime_with_sim();
        reduced.config.appearance.motion = CompanionMotionPreference::Reduced;
        reduced.attachment = Some(WindowAttachment {
            id: ExternalWindowId::from_raw(9),
            top_edge_offset: 10.0,
            perched: true,
        });
        reduced.detach_missing_attachment();
        assert_eq!(
            reduced.simulation.as_ref().unwrap().body.mode,
            BodyMode::Sleeping
        );
    }

    // SDTEST-1519
    #[test]
    fn reduced_motion_release_never_starts_dynamic_frames() {
        let mut runtime = runtime_with_sim();
        runtime.config.appearance.motion = CompanionMotionPreference::Reduced;
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 80.0);
        assert!(!runtime.finish_user_drag_snapshot_for_test(
            Point2::new(100.0, 100.0),
            &[],
            &[display()]
        ));
        assert!(!runtime.simulation.as_ref().unwrap().moving(true));
        assert_ne!(
            runtime.simulation.as_ref().unwrap().body.mode,
            BodyMode::Dynamic
        );
    }

    // SDTEST-1520
    #[test]
    fn subthreshold_drag_jitter_does_not_move_native_overlay() {
        let start = gpui::point(px(100.0), px(100.0));
        let jitter = gpui::point(px(103.0), px(102.0));
        assert!(!drag_crossed_threshold(start, jitter, 1.0));
        assert!(!should_apply_native_origin(false, true));
    }

    // SDTEST-1521
    #[test]
    fn immediate_snap_release_starts_attachment_follow_lifecycle() {
        let mut runtime = runtime_with_sim();
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 140.0);
        let window = external_window(55, bounds(200.0, 300.0, 260.0, 180.0));

        let lifecycle = runtime.finish_user_drag_lifecycle_for_test(
            Point2::new(0.0, 0.0),
            &[window],
            &[display()],
        );

        assert_eq!(lifecycle, ReleaseLifecycle::StartAttachmentFollow);
        assert!(runtime.attachment.is_some());
        assert!(!runtime.attachment_timer_scheduled);
    }

    // SDTEST-1522
    #[test]
    fn floor_landing_policy_schedules_roam_only_when_unattached_and_not_dragging() {
        assert!(should_schedule_roam_after_landing(true, false, false));
        assert!(!should_schedule_roam_after_landing(true, true, false));
        assert!(!should_schedule_roam_after_landing(true, false, true));
        assert!(!should_schedule_roam_after_landing(false, false, false));
    }

    // SDTEST-1523
    #[test]
    fn disappeared_attachment_requests_dynamic_frame_restart_when_falling() {
        let mut runtime = runtime_with_sim();
        assert!(runtime.begin_attachment(
            ExternalWindowId::from_raw(66),
            bounds(200.0, 260.0, 420.0, 240.0),
            &[display()]
        ));

        let outcome = runtime.follow_attached_snapshot(&[], &[display()]);

        assert_eq!(outcome, AttachmentFollowOutcome::dynamic_restart());
        assert_eq!(
            runtime.simulation.as_ref().unwrap().body.mode,
            BodyMode::Dynamic
        );
    }

    // SDTEST-1524
    #[test]
    fn window_climbing_disabled_prevents_snap_platforms_and_window_attachment() {
        let mut runtime = runtime_with_sim();
        runtime.config.appearance.desktop.allow_window_climbing = false;
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 140.0);
        let window = external_window(77, bounds(200.0, 300.0, 260.0, 180.0));

        let lifecycle = runtime.finish_user_drag_lifecycle_for_test(
            Point2::new(0.0, 0.0),
            &[window],
            &[display()],
        );

        assert_eq!(lifecycle, ReleaseLifecycle::RequestPhysicsFrame);
        assert!(runtime.attachment.is_none());
        assert!(runtime.physics_platforms.is_empty());
        let sim = runtime.simulation.as_mut().unwrap();
        for _ in 0..90 {
            if sim
                .step_physics_capped(33, &runtime.physics_platforms)
                .landed
            {
                break;
            }
        }
        assert_eq!(
            sim.body.contact().unwrap().kind,
            WalkableSurfaceKind::ScreenFloor
        );
    }

    // SDTEST-1525
    #[test]
    fn maximized_like_rejects_taskbar_inset_but_keeps_ordinary_large_window() {
        assert!(maximized_like(
            bounds(24.0, 32.0, 752.0, 552.0),
            display().work_area,
            display().scale_factor,
        ));
        assert!(!valid_physics_window(
            bounds(24.0, 32.0, 752.0, 552.0),
            &[display()]
        ));
        assert!(!maximized_like(
            bounds(120.0, 96.0, 560.0, 420.0),
            display().work_area,
            display().scale_factor,
        ));
        assert!(valid_physics_window(
            bounds(120.0, 96.0, 560.0, 420.0),
            &[display()]
        ));
    }

    // SDTEST-1526
    #[test]
    fn stale_mouse_up_velocity_is_zeroed() {
        let now = Instant::now();
        let sampled = Point2::new(300.0, -200.0);
        assert_eq!(
            release_velocity_at_mouse_up(sampled, now, now + DRAG_VELOCITY_SAMPLE_LIMIT),
            sampled
        );
        assert_eq!(
            release_velocity_at_mouse_up(
                sampled,
                now,
                now + DRAG_VELOCITY_SAMPLE_LIMIT + Duration::from_millis(1),
            ),
            Point2::new(0.0, 0.0)
        );
    }

    // SDTEST-1527
    #[test]
    fn disabling_window_climbing_mid_fall_clears_cached_window_platforms() {
        let mut runtime = runtime_with_sim();
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 40.0);
        let window = external_window(88, bounds(200.0, 300.0, 260.0, 180.0));

        assert_eq!(
            runtime.finish_user_drag_lifecycle_for_test(
                Point2::new(0.0, 0.0),
                &[window],
                &[display()],
            ),
            ReleaseLifecycle::RequestPhysicsFrame
        );
        assert_eq!(runtime.physics_platforms.len(), 1);

        runtime.config.appearance.desktop.allow_window_climbing = false;
        runtime.apply_window_climbing_transition(true);

        assert!(runtime.physics_windows.is_empty());
        assert!(runtime.physics_platforms.is_empty());
        let sim = runtime.simulation.as_mut().unwrap();
        for _ in 0..90 {
            if sim
                .step_physics_capped(33, &runtime.physics_platforms)
                .landed
            {
                break;
            }
        }
        assert_eq!(
            sim.body.contact().unwrap().kind,
            WalkableSurfaceKind::ScreenFloor
        );
    }

    // SDTEST-1528
    #[test]
    fn snapped_window_display_becomes_disappearance_fall_floor_context() {
        let primary = DesktopDisplay {
            bounds: bounds(0.0, 0.0, 1600.0, 900.0),
            work_area: bounds(0.0, 0.0, 1600.0, 900.0),
            ..display()
        };
        let portrait = DesktopDisplay {
            bounds: bounds(1600.0, 0.0, 600.0, 3000.0),
            work_area: bounds(1600.0, 0.0, 600.0, 3000.0),
            ..display()
        };
        let displays = [portrait, primary];
        let window = external_window(99, bounds(200.0, 300.0, 360.0, 220.0));
        let mut runtime = runtime_with_sim();
        let sim = runtime.simulation.as_mut().unwrap();
        sim.update_display(portrait);
        sim.simulation.position = Point2::new(220.0, 140.0);

        assert_eq!(
            runtime.finish_user_drag_lifecycle_for_test(
                Point2::new(0.0, 0.0),
                std::slice::from_ref(&window),
                &displays,
            ),
            ReleaseLifecycle::StartAttachmentFollow
        );
        assert_eq!(
            runtime.simulation.as_ref().unwrap().display.work_area,
            primary.work_area
        );

        assert!(runtime.detach_missing_attachment());
        let sim = runtime.simulation.as_mut().unwrap();
        assert_eq!(sim.display.work_area, primary.work_area);
        for _ in 0..90 {
            if sim.step_physics_capped(33, &[]).landed {
                break;
            }
        }

        assert_eq!(
            sim.body.contact().unwrap().kind,
            WalkableSurfaceKind::ScreenFloor
        );
        assert_eq!(
            sim.simulation.position.y,
            raw_rect(primary.work_area).bottom() - sim.extent
        );
        assert!(sim.simulation.position.y < raw_rect(portrait.work_area).bottom() - sim.extent);
    }

    // SDTEST-1529
    #[test]
    fn magnetic_snap_acquires_early_and_releases_only_beyond_hysteresis() {
        let window = external_window(81, bounds(200.0, 300.0, 360.0, 220.0));
        let windows = vec![window.clone()];
        let extent = 160.0;
        let preview_position = Point2::new(230.0, 200.0);

        assert!(choose_snap_window(&windows, &[display()], preview_position, extent).is_none());
        let acquired =
            choose_magnetic_snap_window(&windows, &[display()], preview_position, extent, None)
                .expect("preview should acquire before the strict release threshold");
        assert_eq!(acquired.window.id, window.id);

        let hysteresis_position = Point2::new(230.0, 250.0);
        assert!(choose_magnetic_snap_window(
            &windows,
            &[display()],
            hysteresis_position,
            extent,
            None,
        )
        .is_none());
        assert_eq!(
            choose_magnetic_snap_window(
                &windows,
                &[display()],
                hysteresis_position,
                extent,
                Some(window.id),
            )
            .expect("preferred target should remain sticky inside hysteresis")
            .window
            .id,
            window.id
        );
        assert!(choose_magnetic_snap_window(
            &windows,
            &[display()],
            Point2::new(230.0, 251.0),
            extent,
            Some(window.id),
        )
        .is_none());
    }

    // SDTEST-1530
    #[test]
    fn magnetic_snap_position_lands_on_top_and_preserves_minimum_overlap() {
        let extent = character_window_size(CompanionScale::Medium);
        let candidate = SnapCandidate {
            window: external_window(82, bounds(200.0, 300.0, 300.0, 180.0)),
            display: display(),
            extent,
            min_horizontal_overlap: SNAP_MIN_HORIZONTAL_OVERLAP,
            generation: 82,
        };
        let left = magnetic_snap_position(&candidate, Point2::new(0.0, 190.0));
        assert_eq!(left, Some(Point2::new(76.0, 140.0)));
        let right = magnetic_snap_position(&candidate, Point2::new(700.0, 190.0));
        assert_eq!(right, Some(Point2::new(464.0, 140.0)));
    }

    // SDTEST-1531
    #[test]
    fn active_magnetic_preview_commits_attachment_on_release() {
        let mut runtime = runtime_with_sim();
        let window = external_window(83, bounds(200.0, 300.0, 360.0, 220.0));
        runtime.drag_snap = Some(SnapCandidate {
            window: window.clone(),
            display: display(),
            extent: character_window_size(CompanionScale::Medium),
            min_horizontal_overlap: SNAP_MIN_HORIZONTAL_OVERLAP,
            generation: window.id.raw(),
        });
        runtime.drag_free_position = Some(Point2::new(230.0, 200.0));
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(230.0, 200.0);

        assert_eq!(
            runtime.finish_user_drag_lifecycle_for_test(
                Point2::new(0.0, 0.0),
                std::slice::from_ref(&window),
                &[display()],
            ),
            ReleaseLifecycle::StartAttachmentFollow
        );
        assert_eq!(runtime.attachment.unwrap().id, window.id);
        assert!(runtime.drag_snap.is_none());
        assert_eq!(runtime.simulation.unwrap().simulation.position.y, 140.0);
    }

    // SDTEST-1534
    #[test]
    fn drag_window_snapshots_are_throttled_to_the_refresh_interval() {
        let started = Instant::now();
        assert!(drag_snapshot_refresh_due(
            None,
            started,
            DRAG_WINDOW_LIST_REFRESH_INTERVAL,
        ));
        assert!(!drag_snapshot_refresh_due(
            Some(started),
            started + DRAG_WINDOW_LIST_REFRESH_INTERVAL - Duration::from_millis(1),
            DRAG_WINDOW_LIST_REFRESH_INTERVAL,
        ));
        assert!(drag_snapshot_refresh_due(
            Some(started),
            started + DRAG_WINDOW_LIST_REFRESH_INTERVAL,
            DRAG_WINDOW_LIST_REFRESH_INTERVAL,
        ));
    }

    // SDTEST-1535
    #[test]
    fn runtime_floor_collision_subtracts_companion_extent_exactly_once() {
        let mut runtime = runtime_with_sim();
        runtime.simulation.as_mut().unwrap().simulation.position = Point2::new(220.0, 80.0);
        assert!(runtime.finish_user_drag_snapshot_for_test(
            Point2::new(0.0, 0.0),
            &[],
            &[display()],
        ));
        let sim = runtime.simulation.as_mut().unwrap();
        assert_eq!(sim.physics_bounds(), raw_rect(display().work_area));
        for _ in 0..90 {
            if sim.step_physics_capped(33, &[]).landed {
                break;
            }
        }
        assert_eq!(sim.simulation.position.y, 600.0 - sim.extent);
        assert_eq!(
            sim.body.contact().unwrap().kind,
            WalkableSurfaceKind::ScreenFloor
        );
    }

    // SDTEST-1536
    #[test]
    fn magnetic_release_never_switches_preview_window_id() {
        let preview = external_window(91, bounds(200.0, 300.0, 360.0, 220.0));
        let competitor = external_window(92, bounds(210.0, 292.0, 360.0, 220.0));
        let mut runtime = runtime_with_sim();
        runtime.drag_snap = choose_magnetic_snap_window(
            std::slice::from_ref(&preview),
            &[display()],
            Point2::new(230.0, 200.0),
            runtime.simulation.as_ref().unwrap().logical_extent,
            None,
        );
        runtime.drag_free_position = Some(Point2::new(230.0, 200.0));

        let lifecycle = runtime.finish_user_drag_lifecycle_for_test(
            Point2::new(0.0, 0.0),
            &[competitor, preview.clone()],
            &[display()],
        );

        assert_eq!(lifecycle, ReleaseLifecycle::StartAttachmentFollow);
        assert_eq!(runtime.attachment.unwrap().id, preview.id);
    }

    // SDTEST-1537
    #[test]
    fn missing_preview_window_does_not_fallback_to_another_window() {
        let preview = external_window(93, bounds(200.0, 300.0, 360.0, 220.0));
        let competitor = external_window(94, bounds(210.0, 292.0, 360.0, 220.0));
        let mut runtime = runtime_with_sim();
        runtime.drag_snap = choose_magnetic_snap_window(
            std::slice::from_ref(&preview),
            &[display()],
            Point2::new(230.0, 200.0),
            runtime.simulation.as_ref().unwrap().logical_extent,
            None,
        );
        runtime.drag_free_position = Some(Point2::new(230.0, 200.0));

        let lifecycle = runtime.finish_user_drag_lifecycle_for_test(
            Point2::new(0.0, 0.0),
            &[competitor],
            &[display()],
        );

        assert_eq!(lifecycle, ReleaseLifecycle::RequestPhysicsFrame);
        assert!(runtime.attachment.is_none());
    }

    // SDTEST-1538
    #[test]
    fn preview_release_and_follow_share_the_same_perch_origin() {
        let window = external_window(95, bounds(200.0, 300.0, 130.0, 220.0));
        let mut runtime = runtime_with_sim();
        let free_position = Point2::new(110.0, 200.0);
        let candidate = choose_magnetic_snap_window(
            std::slice::from_ref(&window),
            &[display()],
            free_position,
            runtime.simulation.as_ref().unwrap().logical_extent,
            None,
        )
        .expect("narrow window should remain a valid partial-overlap perch");
        let preview = magnetic_snap_position(&candidate, free_position).unwrap();
        runtime.drag_snap = Some(candidate);
        runtime.drag_free_position = Some(free_position);

        assert_eq!(
            runtime.finish_user_drag_lifecycle_for_test(
                Point2::new(0.0, 0.0),
                std::slice::from_ref(&window),
                &[display()],
            ),
            ReleaseLifecycle::StartAttachmentFollow
        );
        assert_eq!(
            runtime.simulation.as_ref().unwrap().simulation.position,
            preview
        );
        let follow = runtime.follow_attached_snapshot(std::slice::from_ref(&window), &[display()]);
        assert!(follow.attached);
        assert!(!follow.moved);
        assert_eq!(runtime.simulation.unwrap().simulation.position, preview);
    }

    // SDTEST-1539
    #[test]
    fn windows_snap_metrics_scale_extent_and_thresholds_for_target_display() {
        let physical = snap_coordinate_metrics(
            160.0,
            SNAP_PREVIEW_VERTICAL_GAP,
            SNAP_MIN_HORIZONTAL_OVERLAP,
            2.0,
            true,
        );
        let logical = snap_coordinate_metrics(
            160.0,
            SNAP_PREVIEW_VERTICAL_GAP,
            SNAP_MIN_HORIZONTAL_OVERLAP,
            2.0,
            false,
        );

        assert_eq!(physical, (320.0, 144.0, 72.0));
        assert_eq!(logical, (160.0, 72.0, 36.0));
        assert_eq!(scale_coordinate_distance(72.0, 2.0, true), 144.0);
        assert_eq!(scale_coordinate_distance(72.0, 2.0, false), 72.0);
    }

    // SDTEST-1540
    #[test]
    fn drag_snapshot_policy_separates_full_list_and_locked_window_refreshes() {
        let started = Instant::now();
        assert!(!drag_snapshot_refresh_due(
            Some(started),
            started + DRAG_LOCKED_WINDOW_REFRESH_INTERVAL,
            DRAG_WINDOW_LIST_REFRESH_INTERVAL,
        ));
        assert!(drag_snapshot_refresh_due(
            Some(started),
            started + DRAG_LOCKED_WINDOW_REFRESH_INTERVAL,
            DRAG_LOCKED_WINDOW_REFRESH_INTERVAL,
        ));
        assert!(drag_snapshot_refresh_due(
            Some(started),
            started + DRAG_WINDOW_LIST_REFRESH_INTERVAL,
            DRAG_WINDOW_LIST_REFRESH_INTERVAL,
        ));
    }

    // SDTEST-1541
    #[test]
    fn autonomous_climb_ignores_maximized_like_windows() {
        let maximized = external_window(101, bounds(24.0, 32.0, 752.0, 552.0));
        let ordinary = external_window(102, bounds(120.0, 96.0, 560.0, 420.0));
        let selected = choose_external_window(
            &[maximized, ordinary.clone()],
            &[display()],
            Point2::new(400.0, 80.0),
            None,
        )
        .expect("an ordinary unmaximized window should remain climbable");

        assert_eq!(selected.id, ordinary.id);
    }

    // SDTEST-1542
    #[test]
    fn invalidated_preview_blocks_retargeting_for_the_rest_of_the_drag() {
        let preview = external_window(103, bounds(200.0, 300.0, 360.0, 220.0));
        let competitor = external_window(104, bounds(210.0, 292.0, 360.0, 220.0));
        let mut runtime = runtime_with_sim();
        runtime.drag_invalidated_preview_id = Some(preview.id);
        runtime.drag_free_position = Some(Point2::new(230.0, 200.0));

        let lifecycle = runtime.finish_user_drag_lifecycle_for_test(
            Point2::new(0.0, 0.0),
            &[competitor],
            &[display()],
        );

        assert_eq!(lifecycle, ReleaseLifecycle::RequestPhysicsFrame);
        assert!(runtime.attachment.is_none());
    }

    // SDTEST-1543
    #[test]
    fn hysteresis_preview_release_and_follow_use_the_canonical_perch_overlap() {
        let window = external_window(105, bounds(200.0, 300.0, 130.0, 220.0));
        let mut runtime = runtime_with_sim();
        let free_position = Point2::new(55.0, 200.0);
        let candidate = choose_magnetic_snap_window(
            std::slice::from_ref(&window),
            &[display()],
            free_position,
            runtime.simulation.as_ref().unwrap().logical_extent,
            Some(window.id),
        )
        .expect("the locked preview should remain eligible inside the hysteresis overlap");
        let preview = magnetic_snap_position(&candidate, free_position).unwrap();
        assert_eq!(
            candidate.min_horizontal_overlap,
            SNAP_MIN_HORIZONTAL_OVERLAP
        );
        runtime.drag_snap = Some(candidate);
        runtime.drag_free_position = Some(free_position);

        assert_eq!(
            runtime.finish_user_drag_lifecycle_for_test(
                Point2::new(0.0, 0.0),
                std::slice::from_ref(&window),
                &[display()],
            ),
            ReleaseLifecycle::StartAttachmentFollow
        );
        assert_eq!(
            runtime.simulation.as_ref().unwrap().simulation.position,
            preview
        );
        let follow = runtime.follow_attached_snapshot(std::slice::from_ref(&window), &[display()]);
        assert!(follow.attached);
        assert!(!follow.moved);
        assert_eq!(runtime.simulation.unwrap().simulation.position, preview);
    }

    // SDTEST-1544
    #[test]
    fn autonomous_climb_respects_single_monitor_routing() {
        let primary = display();
        let secondary = DesktopDisplay {
            bounds: bounds(800.0, 0.0, 800.0, 600.0),
            work_area: bounds(800.0, 0.0, 800.0, 600.0),
            ..display()
        };
        let local = external_window(106, bounds(300.0, 360.0, 300.0, 180.0));
        let remote = external_window(107, bounds(820.0, 120.0, 300.0, 180.0));

        let selected = choose_external_window(
            &[remote, local.clone()],
            &[primary, secondary],
            Point2::new(760.0, 100.0),
            Some(primary),
        )
        .expect("the current display should retain an eligible autonomous target");

        assert_eq!(selected.id, local.id);
    }

    // SDTEST-1545
    #[test]
    fn autonomous_climb_ranks_vertical_gap_then_horizontal_distance_then_native_id() {
        let vertically_near = external_window(108, bounds(500.0, 120.0, 200.0, 180.0));
        let horizontally_near_high_id = external_window(110, bounds(230.0, 160.0, 200.0, 180.0));
        let horizontally_near_low_id = external_window(109, bounds(230.0, 160.0, 200.0, 180.0));

        let selected = choose_external_window(
            &[
                horizontally_near_high_id,
                vertically_near.clone(),
                horizontally_near_low_id,
            ],
            &[display()],
            Point2::new(300.0, 100.0),
            None,
        )
        .expect("an eligible autonomous target should be selected");

        assert_eq!(selected.id, vertically_near.id);

        let horizontally_near = external_window(111, bounds(230.0, 160.0, 200.0, 180.0));
        let horizontally_far = external_window(112, bounds(560.0, 160.0, 200.0, 180.0));
        let horizontal = choose_external_window(
            &[horizontally_far, horizontally_near.clone()],
            &[display()],
            Point2::new(300.0, 100.0),
            None,
        )
        .unwrap();
        assert_eq!(horizontal.id, horizontally_near.id);

        let tied = choose_external_window(
            &[
                external_window(110, bounds(230.0, 160.0, 200.0, 180.0)),
                external_window(109, bounds(230.0, 160.0, 200.0, 180.0)),
            ],
            &[display()],
            Point2::new(300.0, 100.0),
            None,
        )
        .unwrap();
        assert_eq!(tied.id, ExternalWindowId::from_raw(109));
    }

    // SDTEST-1546
    #[test]
    fn moved_preview_geometry_invalidates_the_drag_without_competitor_fallback() {
        let preview = external_window(113, bounds(200.0, 300.0, 360.0, 220.0));
        let moved_preview = external_window(113, bounds(200.0, 500.0, 360.0, 220.0));
        let competitor = external_window(114, bounds(210.0, 292.0, 360.0, 220.0));
        let mut runtime = runtime_with_sim();
        runtime.drag_snap = choose_magnetic_snap_window(
            std::slice::from_ref(&preview),
            &[display()],
            Point2::new(230.0, 200.0),
            runtime.simulation.as_ref().unwrap().logical_extent,
            None,
        );
        runtime.drag_free_position = Some(Point2::new(230.0, 200.0));
        runtime.drag_windows = vec![moved_preview, competitor.clone()];
        runtime.drag_displays = vec![display()];

        runtime.invalidate_drag_preview_if_geometry_changed(preview.id);

        assert!(runtime.drag_snap.is_none());
        assert_eq!(runtime.drag_invalidated_preview_id, Some(preview.id));
        assert_eq!(
            runtime.finish_user_drag_lifecycle_for_test(
                Point2::new(0.0, 0.0),
                &[competitor],
                &[display()],
            ),
            ReleaseLifecycle::RequestPhysicsFrame
        );
        assert!(runtime.attachment.is_none());
    }
}
