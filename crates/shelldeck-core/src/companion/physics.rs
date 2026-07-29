use super::{Point2, Rect, SurfaceId, WalkableSurface, WalkableSurfaceKind};

const EPSILON: f32 = 0.001;
const WORK_AREA_FLOOR_ID: &str = "work_area:floor";

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
    pub boundary_restitution: f32,
    pub boundary_settle_velocity: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: 2_400.0,
            terminal_velocity: 2_400.0,
            max_horizontal_speed: 1_600.0,
            air_drag: 0.08,
            boundary_restitution: 0.35,
            boundary_settle_velocity: 4.0,
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

        self.validate_contact(platforms, work_area);
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
        let min_x = work_area.x;
        let max_x = (work_area.right() - self.size.x).max(min_x);
        self.position.x = desired.x.clamp(min_x, max_x);
        if desired.x < min_x && self.velocity.x < 0.0 {
            self.velocity.x = reflected_velocity(self.velocity.x, config.boundary_restitution);
            settle_axis(&mut self.velocity.x, config.boundary_settle_velocity);
        } else if desired.x > max_x && self.velocity.x > 0.0 {
            self.velocity.x = -reflected_velocity(self.velocity.x, config.boundary_restitution);
            settle_axis(&mut self.velocity.x, config.boundary_settle_velocity);
        }

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
                id: SurfaceId(WORK_AREA_FLOOR_ID.to_string()),
                generation: 0,
                kind: WalkableSurfaceKind::ScreenFloor,
            });
            landed = true;
        } else if desired.y < work_area.y && self.velocity.y < 0.0 {
            self.position.y = work_area.y;
            self.velocity.y = reflected_velocity(self.velocity.y, config.boundary_restitution);
            settle_axis(&mut self.velocity.y, config.boundary_settle_velocity);
            self.contact = None;
        } else {
            self.position.y = desired.y;
            self.contact = None;
        }

        self.result(previous_position, landed)
    }

    fn validate_contact(&mut self, platforms: &[WalkableSurface], work_area: Rect) {
        let Some(contact) = &self.contact else {
            return;
        };
        if contact.id.0 == WORK_AREA_FLOOR_ID {
            self.validate_floor_contact(work_area);
            return;
        }
        let valid = platforms.iter().any(|surface| {
            surface.id == contact.id && surface.source_generation == contact.generation
        });
        if !valid {
            self.clear_contact();
        }
    }

    fn validate_floor_contact(&mut self, work_area: Rect) {
        let floor_y = work_area.bottom();
        let bottom = self.position.y + self.size.y;
        if (bottom - floor_y).abs() <= EPSILON {
            let min_x = work_area.x;
            let max_x = (work_area.right() - self.size.x).max(min_x);
            self.position.x = self.position.x.clamp(min_x, max_x);
            self.position.y = floor_y - self.size.y;
            self.velocity.y = 0.0;
        } else {
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
    platforms
        .iter()
        .filter(|surface| surface.kind == WalkableSurfaceKind::WindowTop)
        .filter(|surface| {
            let y = surface_y(surface);
            previous_bottom <= y + EPSILON && desired_bottom >= y - EPSILON
        })
        .filter(|surface| {
            let y = surface_y(surface);
            let Some((left, right)) = horizontal_body_interval_at_vertical_toi(
                previous_x,
                current_x,
                width,
                previous_bottom,
                desired_bottom,
                y,
            ) else {
                return false;
            };
            horizontal_overlap(
                left,
                right,
                surface.segment.start.x.min(surface.segment.end.x),
                surface.segment.start.x.max(surface.segment.end.x),
            )
        })
        .min_by(|a, b| compare_landing_surfaces(a, b))
}

fn horizontal_body_interval_at_vertical_toi(
    previous_x: f32,
    current_x: f32,
    width: f32,
    previous_bottom: f32,
    desired_bottom: f32,
    surface_y: f32,
) -> Option<(f32, f32)> {
    let vertical_span = desired_bottom - previous_bottom;
    let toi = if vertical_span.abs() <= EPSILON {
        if (surface_y - previous_bottom).abs() <= EPSILON {
            1.0
        } else {
            return None;
        }
    } else {
        ((surface_y - previous_bottom) / vertical_span).clamp(0.0, 1.0)
    };
    let left = previous_x + (current_x - previous_x) * toi;
    Some((left, left + width))
}

fn compare_landing_surfaces(a: &WalkableSurface, b: &WalkableSurface) -> std::cmp::Ordering {
    surface_y(a)
        .total_cmp(&surface_y(b))
        .then_with(|| a.id.0.cmp(&b.id.0))
        .then_with(|| a.source_generation.cmp(&b.source_generation))
        .then_with(|| a.segment.start.x.total_cmp(&b.segment.start.x))
        .then_with(|| a.segment.end.x.total_cmp(&b.segment.end.x))
        .then_with(|| a.segment.start.y.total_cmp(&b.segment.start.y))
        .then_with(|| a.segment.end.y.total_cmp(&b.segment.end.y))
}

fn horizontal_overlap(left: f32, right: f32, surface_left: f32, surface_right: f32) -> bool {
    left < surface_right - EPSILON && right > surface_left + EPSILON
}

fn surface_y(surface: &WalkableSurface) -> f32 {
    surface.segment.start.y.min(surface.segment.end.y)
}

fn reflected_velocity(velocity: f32, restitution: f32) -> f32 {
    velocity.abs() * restitution.clamp(0.0, 1.0)
}

fn settle_axis(velocity: &mut f32, settle_velocity: f32) {
    if velocity.abs() <= settle_velocity.max(0.0) {
        *velocity = 0.0;
    }
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
            boundary_restitution: 0.5,
            boundary_settle_velocity: 1.0,
        }
    }

    fn config() -> PhysicsConfig {
        PhysicsConfig {
            gravity: 100.0,
            terminal_velocity: 1_200.0,
            max_horizontal_speed: 50.0,
            air_drag: 0.0,
            boundary_restitution: 0.5,
            boundary_settle_velocity: 1.0,
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

    // SDTEST-1532
    #[test]
    fn side_wall_collision_clamps_and_reflects_horizontal_velocity() {
        let mut body = CompanionBody::new(Point2::new(185.0, 40.0), Point2::new(10.0, 10.0));
        body.velocity.x = 40.0;

        let result = body.step(1.0, config(), &[], area());

        assert_eq!(result.position.x, 190.0);
        assert_eq!(result.velocity.x, -20.0);
        assert!(result.contact.is_none());

        body.velocity.x = -1.5;
        let result = body.step(1.0, config(), &[], Rect::new(190.0, 0.0, 10.0, 180.0));

        assert_eq!(result.position.x, 190.0);
        assert_eq!(result.velocity.x, 0.0);
    }

    // SDTEST-1533
    #[test]
    fn ceiling_collision_clamps_and_reflects_upward_velocity_downward() {
        let mut body = CompanionBody::new(Point2::new(20.0, 3.0), Point2::new(10.0, 10.0));
        body.velocity.y = -50.0;

        let result = body.step(0.1, config(), &[], area());

        assert_eq!(result.position.y, 0.0);
        assert_eq!(result.velocity.y, 20.0);
        assert!(!result.landed);
        assert!(result.contact.is_none());

        body.velocity.y = -1.5;
        let result = body.step(0.01, config(), &[], area());

        assert_eq!(result.position.y, 0.0);
        assert_eq!(result.velocity.y, 0.0);
    }

    // SDTEST-1547
    #[test]
    fn diagonal_sweep_does_not_land_on_platform_only_overlapped_by_union_corridor() {
        let mut body = CompanionBody::new(Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
        body.velocity = Point2::new(50.0, 80.0);
        let platform = top("diagonal-miss", 100.0, 40.0, 45.0, 1);
        let open_area = Rect::new(0.0, 0.0, 200.0, 500.0);

        let result = body.step(1.0, config(), &[platform], open_area);

        assert!(!result.landed);
        assert!(result.contact.is_none());
        assert_eq!(result.position, Point2::new(50.0, 180.0));
    }

    // SDTEST-1548
    #[test]
    fn equal_height_platform_selection_is_stable_when_input_order_is_reversed() {
        let a = top("a", 100.0, 20.0, 80.0, 9);
        let b = top("b", 100.0, 20.0, 80.0, 1);
        let mut forward = CompanionBody::new(Point2::new(30.0, 0.0), Point2::new(10.0, 10.0));
        let mut reversed = forward.clone();
        forward.velocity.y = 1_000.0;
        reversed.velocity.y = 1_000.0;

        let forward_result = forward.step(0.2, config(), &[a.clone(), b.clone()], area());
        let reversed_result = reversed.step(0.2, config(), &[b, a], area());

        assert!(forward_result.landed);
        assert!(reversed_result.landed);
        assert_eq!(
            forward_result
                .contact
                .as_ref()
                .map(|contact| contact.id.0.as_str()),
            Some("a")
        );
        assert_eq!(forward_result.contact, reversed_result.contact);
        assert_eq!(forward_result.position, reversed_result.position);
    }

    // SDTEST-1549
    #[test]
    fn expanded_work_area_floor_wakes_sleeping_body_instead_of_leaving_it_suspended() {
        let mut body = CompanionBody::new(Point2::new(30.0, 160.0), Point2::new(10.0, 10.0));
        body.velocity.y = 1_000.0;
        let landed = body.step(0.05, config(), &[], area());
        assert!(landed.landed);
        assert_eq!(landed.position.y, 170.0);
        assert_eq!(body.mode, BodyMode::Sleeping);

        let expanded_area = Rect::new(0.0, 0.0, 200.0, 240.0);
        let result = body.step(0.1, config(), &[], expanded_area);

        assert!(!result.landed);
        assert!(result.position.y > landed.position.y);
        assert!(result.contact.is_none());
        assert_eq!(body.mode, BodyMode::Dynamic);
    }

    // SDTEST-1550
    #[test]
    fn zero_vertical_span_collision_check_is_safe_and_uses_current_horizontal_interval() {
        let hit = top("zero-span-hit", 20.0, 20.0, 40.0, 1);
        let miss = top("zero-span-miss", 20.0, 0.0, 10.0, 1);
        let platforms = vec![miss, hit];

        let result = nearest_descending_platform(0.0, 25.0, 10.0, 20.0, 20.0, &platforms);

        assert_eq!(
            result.map(|surface| surface.id.0.as_str()),
            Some("zero-span-hit")
        );
    }
}
