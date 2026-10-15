use gpui::{point, px, size, Bounds, DisplayId, Pixels, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockMode {
    Attached(DockSide),
    Floating,
}

impl DockMode {
    pub fn rounds_left(self) -> bool {
        self != Self::Attached(DockSide::Left)
    }

    pub fn rounds_right(self) -> bool {
        self != Self::Attached(DockSide::Right)
    }
}

#[derive(Clone, Debug)]
pub struct DockDisplay {
    pub id: DisplayId,
    pub label: String,
    pub work_area: Bounds<Pixels>,
    pub primary: bool,
    /// Windows' positioning API takes physical desktop coordinates; other
    /// backends use logical coordinates. Window resize always uses logical size.
    pub origin_scale: f32,
}

pub fn initial_display(displays: &[DockDisplay]) -> Option<&DockDisplay> {
    initial_display_index(displays.iter().map(|display| display.primary))
        .map(|index| &displays[index])
}

pub fn initial_display_index(primary_flags: impl IntoIterator<Item = bool>) -> Option<usize> {
    let mut first = None;
    for (index, primary) in primary_flags.into_iter().enumerate() {
        first.get_or_insert(index);
        if primary {
            return Some(index);
        }
    }
    first
}

pub fn dock_bounds(area: Bounds<Pixels>, side: DockSide) -> Bounds<Pixels> {
    let width = px(480.0).min(area.size.width);
    Bounds {
        origin: point(
            match side {
                DockSide::Left => area.origin.x,
                DockSide::Right => area.right() - width,
            },
            area.origin.y,
        ),
        size: size(width, area.size.height),
    }
}

pub fn floating_bounds(area: Bounds<Pixels>, previous: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = px(560.0).min(area.size.width);
    let height = px(740.0)
        .min((area.size.height - px(64.0)).max(px(240.0)))
        .min(area.size.height);
    let margin_x = px(32.0).min((area.size.width - width) * 0.5);
    let margin_y = px(32.0).min((area.size.height - height) * 0.5);
    // Pull clear of the attached edge and keep the whole titlebar reachable.
    Bounds {
        origin: point(
            previous
                .origin
                .x
                .max(area.origin.x + margin_x)
                .min(area.right() - width - margin_x),
            area.origin.y + margin_y,
        ),
        size: size(width, height),
    }
}

pub fn native_origin(origin: Point<Pixels>, scale: f32) -> Point<Pixels> {
    origin * scale
}

/// Recognize a compositor's completed edge tile, rather than docking a window
/// merely because it was dragged past a monitor seam. Full-height and a free
/// opposite edge are required; maximized windows are handled separately.
pub fn tiled_side(bounds: Bounds<Pixels>, area: Bounds<Pixels>) -> Option<DockSide> {
    let near = |a: Pixels, b: Pixels| (a - b).abs() <= px(8.0);
    if !near(bounds.origin.y, area.origin.y)
        || !near(bounds.bottom(), area.bottom())
        || bounds.size.width >= area.size.width - px(16.0)
    {
        return None;
    }
    if near(bounds.origin.x, area.origin.x) {
        Some(DockSide::Left)
    } else if near(bounds.right(), area.right()) {
        Some(DockSide::Right)
    } else {
        None
    }
}

/// Every asynchronous hide carries a generation so reopening during the exit
/// animation cannot hide the newly visible window.
#[derive(Default)]
pub struct DockVisibility {
    generation: u64,
    pub closing: bool,
}

impl DockVisibility {
    pub fn show(&mut self) {
        self.generation += 1;
        self.closing = false;
    }

    pub fn close(&mut self) -> u64 {
        self.generation += 1;
        self.closing = true;
        self.generation
    }

    pub fn can_finish_close(&self, generation: u64) -> bool {
        self.closing && self.generation == generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    // SDTEST-1956 — SDUC-434: pointer/array order must not choose another monitor.
    #[test]
    fn initial_dock_uses_primary_display_and_usable_edges_on_either_side() {
        let displays = [
            area(-1600.0, 40.0, 1600.0, 860.0),
            area(0.0, 28.0, 1920.0, 1012.0),
        ];
        assert_eq!(initial_display_index([false, true]), Some(1));
        assert_eq!(initial_display_index([false, false]), Some(0));
        assert_eq!(initial_display_index([]), None);
        for area in displays {
            let left = dock_bounds(area, DockSide::Left);
            let right = dock_bounds(area, DockSide::Right);
            assert_eq!(left.origin, area.origin);
            assert_eq!(right.right(), area.right());
            assert_eq!(right.bottom(), area.bottom());
            assert_eq!(right.size, size(px(480.0), area.size.height));
            let floating = floating_bounds(area, right);
            assert!(area.contains(&floating.origin));
            assert!(floating.right() <= area.right());
            assert!(floating.bottom() <= area.bottom());
            assert!(floating.origin.x > area.origin.x);
            assert!(floating.right() < area.right());
            assert!(floating.size.height < right.size.height);
        }
        let small = area(-320.0, 0.0, 320.0, 220.0);
        assert_eq!(dock_bounds(small, DockSide::Right), small);
        assert_eq!(floating_bounds(small, small).size, small.size);
        assert_eq!(
            native_origin(point(px(-800.0), px(24.0)), 2.0),
            point(px(-1600.0), px(48.0))
        );
    }

    // SDTEST-1957 — SDUC-434: a floating edge crossing is not a completed snap.
    #[test]
    fn native_tiles_change_the_frame_without_treating_maximized_or_crossing_windows_as_docked() {
        let monitor = area(-1600.0, 40.0, 1600.0, 860.0);
        assert_eq!(
            tiled_side(area(-1600.0, 40.0, 800.0, 860.0), monitor),
            Some(DockSide::Left)
        );
        assert_eq!(
            tiled_side(area(-800.0, 40.0, 800.0, 860.0), monitor),
            Some(DockSide::Right)
        );
        assert_eq!(tiled_side(area(-800.0, 120.0, 800.0, 700.0), monitor), None);
        assert_eq!(tiled_side(monitor, monitor), None);
        assert!(DockMode::Floating.rounds_left() && DockMode::Floating.rounds_right());
        assert!(!DockMode::Attached(DockSide::Left).rounds_left());
        assert!(!DockMode::Attached(DockSide::Right).rounds_right());
    }

    // SDTEST-1958 — SDUC-434: close/reopen may race the delayed exit completion.
    #[test]
    fn reopening_cancels_a_pending_hide_without_cancelling_the_next_close() {
        let mut visibility = DockVisibility::default();
        visibility.show();
        let first = visibility.close();
        visibility.show();
        assert!(!visibility.can_finish_close(first));
        let second = visibility.close();
        assert!(!visibility.can_finish_close(first));
        assert!(visibility.can_finish_close(second));
    }
}
