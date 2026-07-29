use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

impl Point2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }
    pub fn is_valid(self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }

    pub fn contains(self, point: Point2) -> bool {
        point.x >= self.x
            && point.x <= self.right()
            && point.y >= self.y
            && point.y <= self.bottom()
    }

    pub fn clamp_point(self, point: Point2) -> Point2 {
        Point2 {
            x: point.x.clamp(self.x, self.right()),
            y: point.y.clamp(self.y, self.bottom()),
        }
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRotation {
    Normal,
    Left,
    Right,
    Inverted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopDisplay {
    pub id: String,
    pub bounds: Rect,
    pub work_area: Rect,
    pub scale_factor: f32,
    pub refresh_rate_millihz: Option<u32>,
    pub rotation: DisplayRotation,
    pub primary: bool,
}

impl DesktopDisplay {
    pub fn sanitize(&self) -> Option<Self> {
        if !self.bounds.is_valid() || !self.work_area.is_valid() || self.scale_factor <= 0.0 {
            return None;
        }
        let mut copy = self.clone();
        copy.work_area.x = copy.work_area.x.max(copy.bounds.x);
        copy.work_area.y = copy.work_area.y.max(copy.bounds.y);
        copy.work_area.width = copy.work_area.right().min(copy.bounds.right()) - copy.work_area.x;
        copy.work_area.height =
            copy.work_area.bottom().min(copy.bounds.bottom()) - copy.work_area.y;
        copy.work_area.is_valid().then_some(copy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LineSegment {
    pub start: Point2,
    pub end: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SurfaceId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalkableSurfaceKind {
    WindowTop,
    WindowLeftEdge,
    WindowRightEdge,
    ScreenFloor,
    ScreenEdge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkableSurface {
    pub id: SurfaceId,
    pub kind: WalkableSurfaceKind,
    pub segment: LineSegment,
    pub normal: Vector2,
    pub source_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalWindowGeometry {
    pub id: String,
    pub bounds: Rect,
    pub visible: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub shell_owned: bool,
    pub transient: bool,
    pub generation: u64,
}

impl ExternalWindowGeometry {
    pub fn accepted(&self, work_areas: &[Rect], show_over_fullscreen: bool) -> bool {
        self.visible
            && !self.minimized
            && !self.shell_owned
            && !self.transient
            && self.bounds.is_valid()
            && (show_over_fullscreen || !self.fullscreen)
            && work_areas.iter().any(|area| area.intersects(self.bounds))
    }

    pub fn walkable_surfaces(&self) -> Vec<WalkableSurface> {
        vec![
            WalkableSurface {
                id: SurfaceId(format!("{}:top", self.id)),
                kind: WalkableSurfaceKind::WindowTop,
                segment: LineSegment {
                    start: Point2::new(self.bounds.x, self.bounds.y),
                    end: Point2::new(self.bounds.right(), self.bounds.y),
                },
                normal: Vector2 { x: 0.0, y: -1.0 },
                source_generation: self.generation,
            },
            WalkableSurface {
                id: SurfaceId(format!("{}:left", self.id)),
                kind: WalkableSurfaceKind::WindowLeftEdge,
                segment: LineSegment {
                    start: Point2::new(self.bounds.x, self.bounds.y),
                    end: Point2::new(self.bounds.x, self.bounds.bottom()),
                },
                normal: Vector2 { x: -1.0, y: 0.0 },
                source_generation: self.generation,
            },
            WalkableSurface {
                id: SurfaceId(format!("{}:right", self.id)),
                kind: WalkableSurfaceKind::WindowRightEdge,
                segment: LineSegment {
                    start: Point2::new(self.bounds.right(), self.bounds.y),
                    end: Point2::new(self.bounds.right(), self.bounds.bottom()),
                },
                normal: Vector2 { x: 1.0, y: 0.0 },
                source_generation: self.generation,
            },
        ]
    }
}

pub fn screen_surfaces(display: &DesktopDisplay, generation: u64) -> Vec<WalkableSurface> {
    let area = display.work_area;
    vec![
        WalkableSurface {
            id: SurfaceId(format!("display:{}:floor", display.id)),
            kind: WalkableSurfaceKind::ScreenFloor,
            segment: LineSegment {
                start: Point2::new(area.x, area.bottom()),
                end: Point2::new(area.right(), area.bottom()),
            },
            normal: Vector2 { x: 0.0, y: -1.0 },
            source_generation: generation,
        },
        WalkableSurface {
            id: SurfaceId(format!("display:{}:left", display.id)),
            kind: WalkableSurfaceKind::ScreenEdge,
            segment: LineSegment {
                start: Point2::new(area.x, area.y),
                end: Point2::new(area.x, area.bottom()),
            },
            normal: Vector2 { x: 1.0, y: 0.0 },
            source_generation: generation,
        },
        WalkableSurface {
            id: SurfaceId(format!("display:{}:right", display.id)),
            kind: WalkableSurfaceKind::ScreenEdge,
            segment: LineSegment {
                start: Point2::new(area.right(), area.y),
                end: Point2::new(area.right(), area.bottom()),
            },
            normal: Vector2 { x: -1.0, y: 0.0 },
            source_generation: generation,
        },
    ]
}

pub fn filter_windows(
    windows: &[ExternalWindowGeometry],
    displays: &[DesktopDisplay],
    show_over_fullscreen: bool,
) -> Vec<ExternalWindowGeometry> {
    let work_areas: Vec<_> = displays
        .iter()
        .filter_map(DesktopDisplay::sanitize)
        .map(|display| display.work_area)
        .collect();
    windows
        .iter()
        .filter(|window| window.accepted(&work_areas, show_over_fullscreen))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_filter_rejects_fullscreen_and_invalid_windows() {
        let display = DesktopDisplay {
            id: "a".to_string(),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            work_area: Rect::new(0.0, 0.0, 100.0, 90.0),
            scale_factor: 1.0,
            refresh_rate_millihz: None,
            rotation: DisplayRotation::Normal,
            primary: true,
        };
        let base = ExternalWindowGeometry {
            id: "w".to_string(),
            bounds: Rect::new(10.0, 10.0, 50.0, 50.0),
            visible: true,
            minimized: false,
            fullscreen: false,
            shell_owned: false,
            transient: false,
            generation: 1,
        };
        assert_eq!(
            filter_windows(
                std::slice::from_ref(&base),
                std::slice::from_ref(&display),
                false
            )
            .len(),
            1
        );
        let hidden = ExternalWindowGeometry {
            visible: false,
            ..base.clone()
        };
        let fullscreen = ExternalWindowGeometry {
            fullscreen: true,
            ..base
        };
        assert!(filter_windows(&[hidden, fullscreen], &[display], false).is_empty());
    }

    #[test]
    fn work_area_clamps_points() {
        let area = Rect::new(10.0, 10.0, 100.0, 50.0);
        assert_eq!(
            area.clamp_point(Point2::new(0.0, 100.0)),
            Point2::new(10.0, 60.0)
        );
    }
}
