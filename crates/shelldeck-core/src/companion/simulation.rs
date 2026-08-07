use super::geometry::{Point2, Rect, SurfaceId, WalkableSurface};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterSimulationState {
    Resting,
    ChoosingTarget,
    Walking,
    Climbing,
    Perched,
    Jumping,
    Flying,
    Landing,
    Recovering,
    ScreenFloor,
    Summoned,
    ReturningToDock,
    Sleeping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationFramePolicy {
    Continuous,
    LowRate,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterSimulationConfig {
    pub fixed_timestep_ms: u64,
    pub max_catch_up_steps: u8,
    pub speed_per_second: f32,
    pub movement_duty_cycle_limit: f32,
    pub cooldown_memory: usize,
}

impl Default for CharacterSimulationConfig {
    fn default() -> Self {
        Self {
            fixed_timestep_ms: 33,
            max_catch_up_steps: 2,
            speed_per_second: 120.0,
            movement_duty_cycle_limit: 0.20,
            cooldown_memory: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterSimulation {
    pub state: CharacterSimulationState,
    pub position: Point2,
    pub display_id: String,
    pub target: Option<Point2>,
    pub surface: Option<SurfaceId>,
    pub surface_generation: u64,
    pub elapsed_ms: u64,
    pub moving_ms: u64,
    pub last_actions: VecDeque<String>,
    pub config: CharacterSimulationConfig,
}

impl CharacterSimulation {
    pub fn new(display_id: impl Into<String>, position: Point2) -> Self {
        Self {
            state: CharacterSimulationState::Resting,
            position,
            display_id: display_id.into(),
            target: None,
            surface: None,
            surface_generation: 0,
            elapsed_ms: 0,
            moving_ms: 0,
            last_actions: VecDeque::new(),
            config: CharacterSimulationConfig::default(),
        }
    }

    pub fn request_animation_frames(&self, reduced_motion: bool) -> AnimationFramePolicy {
        if reduced_motion
            || matches!(
                self.state,
                CharacterSimulationState::Resting
                    | CharacterSimulationState::Sleeping
                    | CharacterSimulationState::Perched
            )
        {
            AnimationFramePolicy::None
        } else if matches!(self.state, CharacterSimulationState::ChoosingTarget) {
            AnimationFramePolicy::LowRate
        } else {
            AnimationFramePolicy::Continuous
        }
    }

    pub fn set_target(&mut self, target: Point2) {
        self.target = Some(target);
        self.state = CharacterSimulationState::Walking;
    }

    pub fn step_capped(&mut self, elapsed_ms: u64, bounds: Rect) -> u8 {
        let step = self.config.fixed_timestep_ms.max(1);
        let requested = elapsed_ms / step;
        let steps = requested.min(self.config.max_catch_up_steps as u64) as u8;
        for _ in 0..steps {
            self.step_once(step, bounds);
        }
        steps
    }

    pub fn step_once(&mut self, delta_ms: u64, bounds: Rect) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
        if let Some(target) = self.target {
            let dx = target.x - self.position.x;
            let dy = target.y - self.position.y;
            let distance = (dx * dx + dy * dy).sqrt();
            let max_distance = self.config.speed_per_second * (delta_ms as f32 / 1_000.0);
            if distance <= max_distance || distance <= f32::EPSILON {
                self.position = bounds.clamp_point(target);
                self.target = None;
                self.state = CharacterSimulationState::Resting;
            } else {
                self.position = bounds.clamp_point(Point2::new(
                    self.position.x + dx / distance * max_distance,
                    self.position.y + dy / distance * max_distance,
                ));
                self.moving_ms = self.moving_ms.saturating_add(delta_ms);
            }
        } else if matches!(self.state, CharacterSimulationState::Sleeping) {
            self.position = bounds.clamp_point(self.position);
        }
    }

    pub fn duty_cycle(&self) -> f32 {
        if self.elapsed_ms == 0 {
            0.0
        } else {
            self.moving_ms as f32 / self.elapsed_ms as f32
        }
    }

    pub fn can_start_action(&self) -> bool {
        self.duty_cycle() < self.config.movement_duty_cycle_limit
    }

    pub fn advance_idle_time(&mut self, elapsed_ms: u64) {
        if self.target.is_none() {
            self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
        }
    }

    pub fn remember_action(&mut self, action_id: impl Into<String>) {
        self.last_actions.push_back(action_id.into());
        while self.last_actions.len() > self.config.cooldown_memory {
            self.last_actions.pop_front();
        }
    }

    pub fn action_on_cooldown(&self, action_id: &str) -> bool {
        self.last_actions.iter().any(|recent| recent == action_id)
    }

    pub fn validate_surface(&mut self, surfaces: &[WalkableSurface]) -> bool {
        let Some(surface_id) = &self.surface else {
            return true;
        };
        let valid = surfaces.iter().any(|surface| {
            surface.id == *surface_id && surface.source_generation == self.surface_generation
        });
        if !valid {
            self.surface = None;
            self.target = None;
            self.state = CharacterSimulationState::Recovering;
        }
        valid
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub const fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }
    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }
    pub fn choose_index(&mut self, len: usize) -> Option<usize> {
        (len > 0).then(|| self.next_u32() as usize % len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion::geometry::{LineSegment, Vector2, WalkableSurfaceKind};

    // SDTEST-1472
    #[test]
    fn movement_stays_inside_work_area_and_catch_up_is_capped() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut sim = CharacterSimulation::new("a", Point2::new(50.0, 50.0));
        sim.set_target(Point2::new(500.0, 500.0));
        let steps = sim.step_capped(10_000, bounds);
        assert_eq!(steps, 2);
        assert!(bounds.contains(sim.position));
    }

    // SDTEST-1473
    #[test]
    fn reduced_motion_and_sleeping_request_no_frames() {
        let mut sim = CharacterSimulation::new("a", Point2::new(0.0, 0.0));
        sim.state = CharacterSimulationState::Sleeping;
        assert_eq!(
            sim.request_animation_frames(false),
            AnimationFramePolicy::None
        );
        sim.state = CharacterSimulationState::Walking;
        assert_eq!(
            sim.request_animation_frames(true),
            AnimationFramePolicy::None
        );
    }

    // SDTEST-1474
    #[test]
    fn stale_surface_moves_to_recovering() {
        let mut sim = CharacterSimulation::new("a", Point2::new(0.0, 0.0));
        sim.surface = Some(SurfaceId("w:top".to_string()));
        sim.surface_generation = 1;
        let current = WalkableSurface {
            id: SurfaceId("w:top".to_string()),
            kind: WalkableSurfaceKind::WindowTop,
            segment: LineSegment {
                start: Point2::new(0.0, 0.0),
                end: Point2::new(10.0, 0.0),
            },
            normal: Vector2 { x: 0.0, y: -1.0 },
            source_generation: 2,
        };
        assert!(!sim.validate_surface(&[current]));
        assert_eq!(sim.state, CharacterSimulationState::Recovering);
    }

    // SDTEST-1475
    #[test]
    fn seeded_random_source_is_deterministic_and_cooldowns_work() {
        let mut a = DeterministicRng::seeded(42);
        let mut b = DeterministicRng::seeded(42);
        assert_eq!(a.choose_index(10), b.choose_index(10));
        let mut sim = CharacterSimulation::new("a", Point2::new(0.0, 0.0));
        sim.config.cooldown_memory = 2;
        sim.remember_action("walk");
        sim.remember_action("hop");
        sim.remember_action("sleep");
        assert!(!sim.action_on_cooldown("walk"));
        assert!(sim.action_on_cooldown("sleep"));
    }

    // SDTEST-1476
    #[test]
    fn duty_cycle_blocks_excessive_movement() {
        let mut sim = CharacterSimulation::new("a", Point2::new(0.0, 0.0));
        sim.elapsed_ms = 10_000;
        sim.moving_ms = 3_000;
        assert!(!sim.can_start_action());
        sim.advance_idle_time(10_000);
        assert!(sim.can_start_action());
    }
}
