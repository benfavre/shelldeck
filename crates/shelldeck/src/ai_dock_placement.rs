//! Display inventory for the assistant. X11 exposes root screens through GPUI,
//! so use the existing optional XRandR path for physical monitor boundaries.
use gpui::App;
#[cfg(target_os = "linux")]
use gpui::Pixels;
use shelldeck_ui::ai_dock::layout::DockDisplay;

pub fn positioning_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::companion_desktop::is_x11_session()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

pub fn displays(cx: &App, scale_hint: f32) -> Vec<DockDisplay> {
    let primary = cx.primary_display().map(|display| display.id());
    let fallback: Vec<_> = cx
        .displays()
        .into_iter()
        .map(|display| DockDisplay {
            id: display.id(),
            label: String::new(),
            work_area: display.work_area(),
            primary: Some(display.id()) == primary,
            origin_scale: if cfg!(target_os = "windows") {
                display.scale_factor()
            } else {
                1.0
            },
        })
        .collect();
    #[cfg(target_os = "linux")]
    if crate::companion_desktop::is_x11_session() {
        if let Some(platform) = fallback.first() {
            let output = std::process::Command::new("xrandr")
                .arg("--listactivemonitors")
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    let work_area = super::x11_workarea(scale_hint);
                    let monitors = parse_xrandr_displays(
                        &String::from_utf8_lossy(&output.stdout),
                        scale_hint,
                        work_area,
                    );
                    if !monitors.is_empty() {
                        return monitors
                            .into_iter()
                            .map(|monitor| DockDisplay {
                                id: platform.id,
                                label: monitor.label,
                                work_area: monitor.work_area,
                                primary: monitor.primary,
                                origin_scale: 1.0,
                            })
                            .collect();
                    }
                    tracing::warn!(
                        "xrandr returned no assistant monitor geometry; using GPUI displays"
                    );
                }
                _ => tracing::warn!(
                    "could not query assistant monitors with xrandr; using GPUI displays"
                ),
            }
        }
    }
    let _ = scale_hint;
    fallback
}

#[cfg(target_os = "linux")]
struct XrandrMonitor {
    label: String,
    work_area: gpui::Bounds<Pixels>,
    primary: bool,
}

#[cfg(target_os = "linux")]
fn parse_xrandr_displays(
    output: &str,
    scale: f32,
    work_area: Option<gpui::Bounds<Pixels>>,
) -> Vec<XrandrMonitor> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let words: Vec<_> = line.split_whitespace().collect();
            let bounds = words
                .iter()
                .find_map(|word| super::parse_xrandr_monitor_geometry(word, scale))?;
            let work_area = work_area
                .filter(|area| area.intersects(&bounds))
                .map(|area| bounds.intersect(&area))
                .filter(|area| area.size.width > gpui::px(0.0) && area.size.height > gpui::px(0.0))
                .unwrap_or(bounds);
            Some(XrandrMonitor {
                label: words.last()?.to_string(),
                work_area,
                primary: words.get(1)?.contains('*'),
            })
        })
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use gpui::{point, px, size, Bounds};

    // SDTEST-1959 — SDUC-434: root-screen IDs must not merge physical monitors.
    #[test]
    fn xrandr_dock_inventory_retains_primary_negative_origins_and_work_area() {
        let work_area = Bounds::new(point(px(-1600.0), px(28.0)), size(px(3520.0), px(1012.0)));
        let monitors = parse_xrandr_displays("Monitors: 2\n 0: +DP-1 1600/400x900/220-1600+0 DP-1\n 1: +*HDMI-1 1920/500x1080/280+0+0 HDMI-1\n", 1.0, Some(work_area));
        assert_eq!(monitors.len(), 2);
        assert!(!monitors[0].primary);
        assert_eq!(monitors[0].work_area.origin, point(px(-1600.0), px(28.0)));
        assert_eq!(monitors[0].work_area.size.height, px(872.0));
        assert_eq!(monitors[1].label, "HDMI-1");
        assert!(monitors[1].primary);
        assert_eq!(monitors[1].work_area.bottom(), px(1040.0));
        assert_eq!(
            shelldeck_ui::ai_dock::layout::initial_display_index(
                monitors.iter().map(|monitor| monitor.primary)
            ),
            Some(1)
        );
    }
}
