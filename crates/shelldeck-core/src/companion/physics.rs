use super::{Point2, Rect, SurfaceId, WalkableSurface, WalkableSurfaceKind};

const EPSILON: f32 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    Dynamic,
    Kinematic,
    Sleeping,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsConfig {
    pub gravity: f32,
    pub terminal_velocity: f32,
    pub max_horizontal_speed: f32,
    pub air_drag: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: 2_400.0,
            terminal_velocity: 2_400.0,
            max_horizontal_speed: 1_600.0,
            air_drag: 0.08,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceContact {
    pub id: SurfaceId,
    pub generation: u64,
    pub kind: WalkableSurfaceKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompanionBody {
    pub position: Point2,
    pub size: Point2,
    pub velocity: Point2,
    pub mode: BodyMode,
    contact: Option<SurfaceContact>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub previous_position: Point2,
    pub position: Point2,
    pub velocity: Point2,
    pub moved: bool,
    pub landed: bool,
    pub contact: Option<SurfaceContact>,
}

impl CompanionBody {
    pub fn new(position: Point2, size: Point2) -> Self {
        Self {
            position,
            size,
            velocity: Point2::new(0.0, 0.0),
            mode: BodyMode::Dynamic,
            contact: None,
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(self.position.x, self.position.y, self.size.x, self.size.y)
    }

    pub fn contact(&self) -> Option<&SurfaceContact> {
        self.contact.as_ref()
    }

    pub fn clear_contact(&mut self) {
        self.contact = None;
        if self.mode == BodyMode::Sleeping {
            self.mode = BodyMode::Dynamic;
        }
    }

    pub fn invalidate_contact(&mut self, id: &SurfaceId, generation: u64) -> bool {
        let stale = self
            .contact
            .as_ref()
            .is_some_and(|contact| &contact.id == id && contact.generation != generation);
        if stale {
            self.clear_contact();
        }
        stale
    }

    pub fn release_from_drag(&mut self, velocity: Point2, config: PhysicsConfig) {
        self.velocity = Point2::new(
            velocity
                .x
                .clamp(-config.max_horizontal_speed, config.max_horizontal_speed),
            velocity
                .y
                .clamp(-config.terminal_velocity, config.terminal_velocity),
        );
        self.mode = BodyMode::Dynamic;
        self.contact = None;
    }

    pub fn snap_to_surface(&mut self, surface: &WalkableSurface) {
        self.position.y = surface_y(surface) - self.size.y;
        self.velocity.y = 0.0;
        self.mode = BodyMode::Sleeping;
        self.contact = Some(contact_for(surface));
    }

    pub fn step(
        &mut self,
        dt_seconds: f32,
        config: PhysicsConfig,
        platforms: &[WalkableSurface],
        work_area: Rect,
    ) -> StepResult {
        let previous_position = self.position;
        if dt_seconds <= 0.0 || !dt_seconds.is_finite() {
            return self.result(previous_position, false);
        }

        self.validate_contact(platforms);
        if self.mode == BodyMode::Sleeping && self.contact.is_some() {
            return self.result(previous_position, false);
        }

        if self.mode == BodyMode::Dynamic {
            self.velocity.y = (self.velocity.y + config.gravity * dt_seconds)
                .min(config.terminal_velocity.max(0.0));
            let drag = (1.0 - config.air_drag * dt_seconds).clamp(0.0, 1.0);
            self.velocity.x *= drag;
        }
        self.velocity.x = self
            .velocity
            .x
            .clamp(-config.max_horizontal_speed, config.max_horizontal_speed);

        let desired = Point2::new(
            self.position.x + self.velocity.x * dt_seconds,
            self.position.y + self.velocity.y * dt_seconds,
        );
        self.position.x = desired.x.clamp(
            work_area.x,
            (work_area.right() - self.size.x).max(work_area.x),
        );

        let previous_bottom = previous_position.y + self.size.y;
        let desired_bottom = desired.y + self.size.y;
        let landing = if self.velocity.y >= 0.0 {
            nearest_descending_platform(
                previous_position.x,
                self.position.x,
                self.size.x,
                previous_bottom,
                desired_bottom,
                platforms,
            )
        } else {
            None
        };

        let floor_y = work_area.bottom();
        let floor_hit =
            self.velocity.y >= 0.0 && previous_bottom <= floor_y && desired_bottom >= floor_y;

        let mut landed = false;
        if let Some(surface) = landing {
            self.position.y = surface_y(surface) - self.size.y;
            self.velocity.y = 0.0;
            self.mode = BodyMode::Sleeping;
            self.contact = Some(contact_for(surface));
            landed = true;
        } else if floor_hit || desired_bottom > floor_y {
            self.position.y = floor_y - self.size.y;
            self.velocity.y = 0.0;
            self.mode = BodyMode::Sleeping;
            self.contact = Some(SurfaceContact {
                id: SurfaceId("work_area:floor".to_string()),
                generation: 0,
                kind: WalkableSurfaceKind::ScreenFloor,
            });
            landed = true;
        } else {
            self.position.y = desired.y;
            self.contact = None;
        }

        self.result(previous_position, landed)
    }

    fn validate_contact(&mut self, platforms: &[WalkableSurface]) {
        let Some(contact) = &self.contact else {
            return;
        };
        if contact.id.0 == "work_area:floor" {
            return;
        }
        let valid = platforms.iter().any(|surface| {
            surface.id == contact.id && surface.source_generation == contact.generation
        });
        if !valid {
            self.clear_contact();
        }
    }

    fn result(&self, previous_position: Point2, landed: bool) -> StepResult {
        StepResult {
            previous_position,
            position: self.position,
            velocity: self.velocity,
            moved: (self.position.x - previous_position.x).abs() > EPSILON
                || (self.position.y - previous_position.y).abs() > EPSILON,
            landed,
            contact: self.contact.clone(),
        }
    }
}

fn nearest_descending_platform(
    previous_x: f32,
    current_x: f32,
    width: f32,
    previous_bottom: f32,
    desired_bottom: f32,
    platforms: &[WalkableSurface],
) -> Option<&WalkableSurface> {
    let left = previous_x.min(current_x);
    let right = previous_x.max(current_x) + width;
    platforms
        .iter()
        .filter(|surface| surface.kind == WalkableSurfaceKind::WindowTop)
        .filter(|surface| {
            let y = surface_y(surface);
            previous_bottom <= y + EPSILON && desired_bottom >= y - EPSILON
        })
        .filter(|surface| {
            horizontal_overlap(left, right, surface.segment.start.x, surface.segment.end.x)
        })
        .min_by(|a, b| surface_y(a).total_cmp(&surface_y(b)))
}

fn horizontal_overlap(left: f32, right: f32, surface_left: f32, surface_right: f32) -> bool {
    left < surface_right - EPSILON && right > surface_left + EPSILON
}

fn surface_y(surface: &WalkableSurface) -> f32 {
    surface.segment.start.y.min(surface.segment.end.y)
}

fn contact_for(surface: &WalkableSurface) -> SurfaceContact {
    SurfaceContact {
        id: surface.id.clone(),
        generation: surface.source_generation,
        kind: surface.kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion::{LineSegment, Vector2};

    fn release_config() -> PhysicsConfig {
        PhysicsConfig {
            gravity: 100.0,
            terminal_velocity: 300.0,
            max_horizontal_speed: 50.0,
            air_drag: 0.0,
        }
    }

    fn config() -> PhysicsConfig {
        PhysicsConfig {
            gravity: 100.0,
            terminal_velocity: 1_200.0,
            max_horizontal_speed: 50.0,
            air_drag: 0.0,
        }
    }

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 180.0)
    }

    fn top(id: &str, y: f32, x1: f32, x2: f32, generation: u64) -> WalkableSurface {
        WalkableSurface {
            id: SurfaceId(id.to_string()),
            kind: WalkableSurfaceKind::WindowTop,
            segment: LineSegment {
                start: Point2::new(x1, y),
                end: Point2::new(x2, y),
            },
            normal: Vector2 { x: 0.0, y: -1.0 },
            source_generation: generation,
        }
    }

    // SDTEST-1505
    #[test]
    fn gravity_accelerates_dynamic_body_and_clamps_terminal_velocity() {
        let mut body = CompanionBody::new(Point2::new(10.0, 10.0), Point2::new(10.0, 10.0));
        let open_area = Rect::new(0.0, 0.0, 200.0, 50_000.0);

        let result = body.step(1.0, config(), &[], open_area);
        assert_eq!(result.velocity.y, 100.0);
        assert_eq!(result.position.y, 110.0);
        assert!(!result.landed);
        assert!(result.contact.is_none());

        let result = body.step(20.0, config(), &[], open_area);
        assert_eq!(result.velocity.y, config().terminal_velocity);
        assert!(!result.landed);
        assert!(result.position.y < open_area.bottom() - body.size.y);
    }

    // SDTEST-1506
    #[test]
    fn swept_descending_collision_does_not_tunnel_through_window_top() {
        let mut body = CompanionBody::new(Point2::new(30.0, 0.0), Point2::new(10.0, 10.0));
        body.velocity.y = 1_000.0;
        let platform = top("window:top", 100.0, 20.0, 80.0, 7);
        let result = body.step(0.2, config(), &[platform], area());
        assert!(result.landed);
        assert_eq!(result.position.y, 90.0);
        assert_eq!(result.contact.as_ref().map(|c| c.generation), Some(7));
    }

    // SDTEST-1507
    #[test]
    fn descending_collision_selects_nearest_crossed_platform() {
        let mut body = CompanionBody::new(Point2::new(30.0, 0.0), Point2::new(10.0, 10.0));
        body.velocity.y = 1_000.0;
        let lower = top("lower", 130.0, 20.0, 80.0, 1);
        let upper = top("upper", 80.0, 20.0, 80.0, 2);
        let result = body.step(0.2, config(), &[lower, upper], area());
        assert_eq!(result.position.y, 70.0);
        assert_eq!(
            result.contact.as_ref().map(|c| c.id.0.as_str()),
            Some("upper")
        );
    }

    // SDTEST-1508
    #[test]
    fn platforms_without_horizontal_overlap_are_rejected() {
        let mut body = CompanionBody::new(Point2::new(120.0, 0.0), Point2::new(10.0, 10.0));
        body.velocity.y = 1_000.0;
        let platform = top("miss", 80.0, 20.0, 80.0, 1);
        let result = body.step(0.09, config(), &[platform], area());
        assert!(!result.landed);
        assert!(result.contact.is_none());
        assert!(result.position.y > 80.0);
    }

    // SDTEST-1509
    #[test]
    fn display_work_area_floor_is_used_when_no_platform_matches() {
        let mut body = CompanionBody::new(Point2::new(30.0, 160.0), Point2::new(10.0, 10.0));
        body.velocity.y = 1_000.0;
        let result = body.step(0.05, config(), &[], area());
        assert!(result.landed);
        assert_eq!(result.position.y, 170.0);
        assert_eq!(
            result.contact.as_ref().map(|contact| contact.kind),
            Some(WalkableSurfaceKind::ScreenFloor)
        );
    }

    // SDTEST-1510
    #[test]
    fn release_from_drag_bounds_velocity_and_clears_contact() {
        let mut body = CompanionBody::new(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
        let platform = top("hold", 20.0, 0.0, 50.0, 4);
        body.snap_to_surface(&platform);
        body.release_from_drag(Point2::new(500.0, -900.0), release_config());
        assert_eq!(body.mode, BodyMode::Dynamic);
        assert_eq!(body.velocity, Point2::new(50.0, -300.0));
        assert!(body.contact().is_none());
    }

    // SDTEST-1511
    #[test]
    fn repeated_steps_are_deterministic_and_stale_contacts_invalidate() {
        let platform = top("stable", 100.0, 20.0, 80.0, 1);
        let mut a = CompanionBody::new(Point2::new(30.0, 0.0), Point2::new(10.0, 10.0));
        let mut b = a.clone();
        a.velocity = Point2::new(7.0, 80.0);
        b.velocity = Point2::new(7.0, 80.0);
        let ra = a.step(0.25, config(), std::slice::from_ref(&platform), area());
        let rb = b.step(0.25, config(), std::slice::from_ref(&platform), area());
        assert_eq!(ra, rb);

        a.snap_to_surface(&platform);
        assert!(a.invalidate_contact(&platform.id, 2));
        assert_eq!(a.mode, BodyMode::Dynamic);
        assert!(a.contact().is_none());
    }
}
