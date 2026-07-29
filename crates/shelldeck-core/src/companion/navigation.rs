use super::geometry::{DesktopDisplay, Point2, Rect};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRouteKind {
    SharedEdge,
    NearestEdge,
    Portal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayRoute {
    pub from_display_id: String,
    pub to_display_id: String,
    pub kind: DisplayRouteKind,
    pub exit: Point2,
    pub entry: Point2,
}

pub fn primary_display(displays: &[DesktopDisplay]) -> Option<&DesktopDisplay> {
    displays
        .iter()
        .find(|display| display.primary)
        .or_else(|| displays.first())
}

pub fn recover_display<'a>(
    preferred_id: Option<&str>,
    displays: &'a [DesktopDisplay],
) -> Option<&'a DesktopDisplay> {
    preferred_id
        .and_then(|id| displays.iter().find(|display| display.id == id))
        .or_else(|| primary_display(displays))
}

pub fn clamp_to_available_work_area(
    position: Point2,
    preferred_id: Option<&str>,
    displays: &[DesktopDisplay],
) -> Option<(String, Point2)> {
    if let Some(display) =
        preferred_id.and_then(|id| displays.iter().find(|display| display.id == id))
    {
        return Some((display.id.clone(), display.work_area.clamp_point(position)));
    }
    displays
        .iter()
        .find(|display| display.work_area.contains(position))
        .or_else(|| primary_display(displays))
        .map(|display| (display.id.clone(), display.work_area.clamp_point(position)))
}

pub fn route_between(from: &DesktopDisplay, to: &DesktopDisplay) -> DisplayRoute {
    if let Some((exit, entry)) = shared_edge_points(from.work_area, to.work_area) {
        return DisplayRoute {
            from_display_id: from.id.clone(),
            to_display_id: to.id.clone(),
            kind: DisplayRouteKind::SharedEdge,
            exit,
            entry,
        };
    }
    let horizontal_gap =
        from.work_area.right() < to.work_area.x || to.work_area.right() < from.work_area.x;
    let vertical_gap =
        from.work_area.bottom() < to.work_area.y || to.work_area.bottom() < from.work_area.y;
    let kind = if horizontal_gap && vertical_gap {
        DisplayRouteKind::Portal
    } else {
        DisplayRouteKind::NearestEdge
    };
    let from_center = Point2::new(
        from.work_area.x + from.work_area.width / 2.0,
        from.work_area.y + from.work_area.height / 2.0,
    );
    let to_center = Point2::new(
        to.work_area.x + to.work_area.width / 2.0,
        to.work_area.y + to.work_area.height / 2.0,
    );
    DisplayRoute {
        from_display_id: from.id.clone(),
        to_display_id: to.id.clone(),
        kind,
        exit: nearest_edge_point(from.work_area, to_center),
        entry: nearest_edge_point(to.work_area, from_center),
    }
}

fn shared_edge_points(a: Rect, b: Rect) -> Option<(Point2, Point2)> {
    let epsilon = 1.0;
    if (a.right() - b.x).abs() <= epsilon || (b.right() - a.x).abs() <= epsilon {
        let y1 = a.y.max(b.y);
        let y2 = a.bottom().min(b.bottom());
        if y2 >= y1 {
            let y = (y1 + y2) / 2.0;
            if (a.right() - b.x).abs() <= epsilon {
                return Some((Point2::new(a.right(), y), Point2::new(b.x, y)));
            }
            return Some((Point2::new(a.x, y), Point2::new(b.right(), y)));
        }
    }
    if (a.bottom() - b.y).abs() <= epsilon || (b.bottom() - a.y).abs() <= epsilon {
        let x1 = a.x.max(b.x);
        let x2 = a.right().min(b.right());
        if x2 >= x1 {
            let x = (x1 + x2) / 2.0;
            if (a.bottom() - b.y).abs() <= epsilon {
                return Some((Point2::new(x, a.bottom()), Point2::new(x, b.y)));
            }
            return Some((Point2::new(x, a.y), Point2::new(x, b.bottom())));
        }
    }
    None
}

fn nearest_edge_point(rect: Rect, target: Point2) -> Point2 {
    let clamped = rect.clamp_point(target);
    let distances = [
        (target.x - rect.x).abs(),
        (target.x - rect.right()).abs(),
        (target.y - rect.y).abs(),
        (target.y - rect.bottom()).abs(),
    ];
    let min = distances
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(index, _)| index)
        .unwrap_or(0);
    match min {
        0 => Point2::new(rect.x, clamped.y),
        1 => Point2::new(rect.right(), clamped.y),
        2 => Point2::new(clamped.x, rect.y),
        _ => Point2::new(clamped.x, rect.bottom()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion::geometry::{DisplayRotation, Rect};

    fn display(id: &str, x: f32, y: f32, scale: f32) -> DesktopDisplay {
        DesktopDisplay {
            id: id.to_string(),
            bounds: Rect::new(x, y, 100.0, 100.0),
            work_area: Rect::new(x, y, 100.0, 90.0),
            scale_factor: scale,
            refresh_rate_millihz: None,
            rotation: DisplayRotation::Normal,
            primary: id == "a",
        }
    }

    #[test]
    fn shared_edge_route_prefers_overlap() {
        let route = route_between(&display("a", 0.0, 0.0, 1.0), &display("b", 100.0, 0.0, 2.0));
        assert_eq!(route.kind, DisplayRouteKind::SharedEdge);
        assert_eq!(route.exit.x, 100.0);
        assert_eq!(route.entry.x, 100.0);
    }

    #[test]
    fn disconnected_displays_use_portal_route() {
        let route = route_between(
            &display("a", 0.0, 0.0, 1.0),
            &display("b", 300.0, 300.0, 1.5),
        );
        assert_eq!(route.kind, DisplayRouteKind::Portal);
    }

    #[test]
    fn removed_monitor_recovers_to_primary_work_area() {
        let displays = vec![display("a", 0.0, 0.0, 1.0)];
        let (id, point) =
            clamp_to_available_work_area(Point2::new(500.0, 500.0), Some("gone"), &displays)
                .unwrap();
        assert_eq!(id, "a");
        assert!(displays[0].work_area.contains(point));
    }
}
