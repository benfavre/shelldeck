use itertools::Itertools;
use smallvec::SmallVec;
use std::rc::Rc;
use util::ResultExt;
use uuid::Uuid;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI,
        },
    },
    core::*,
};

use crate::{
    Bounds, DesktopDisplayMetrics, DevicePixels, DisplayId, Pixels, PlatformDisplay,
    logical_point, point, px, size,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowsDisplay {
    pub handle: HMONITOR,
    pub display_id: DisplayId,
    scale_factor: f32,
    bounds: Bounds<Pixels>,
    work_area: Bounds<Pixels>,
    physical_bounds: Bounds<DevicePixels>,
    physical_work_area: Bounds<DevicePixels>,
    uuid: Uuid,
}

// The `HMONITOR` is thread-safe.
unsafe impl Send for WindowsDisplay {}
unsafe impl Sync for WindowsDisplay {}

impl WindowsDisplay {
    pub(crate) fn new(display_id: DisplayId) -> Option<Self> {
        let screen = available_monitors().into_iter().nth(display_id.0 as _)?;
        let info = get_monitor_info(screen).log_err()?;
        let monitor_size = info.monitorInfo.rcMonitor;
        let monitor_work_area = info.monitorInfo.rcWork;
        let uuid = generate_uuid(&info.szDevice);
        let scale_factor = get_scale_factor_for_monitor(screen).log_err()?;
        let physical_size = size(
            (monitor_size.right - monitor_size.left).into(),
            (monitor_size.bottom - monitor_size.top).into(),
        );
        let physical_work_area = rect_device_bounds(monitor_work_area);

        Some(WindowsDisplay {
            handle: screen,
            display_id,
            scale_factor,
            bounds: Bounds {
                origin: logical_point(
                    monitor_size.left as f32,
                    monitor_size.top as f32,
                    scale_factor,
                ),
                size: physical_size.to_pixels(scale_factor),
            },
            work_area: device_bounds_to_logical(physical_work_area, scale_factor),
            physical_bounds: Bounds {
                origin: point(monitor_size.left.into(), monitor_size.top.into()),
                size: physical_size,
            },
            physical_work_area,
            uuid,
        })
    }

    pub fn new_with_handle(monitor: HMONITOR) -> Self {
        let info = get_monitor_info(monitor).expect("unable to get monitor info");
        let monitor_size = info.monitorInfo.rcMonitor;
        let monitor_work_area = info.monitorInfo.rcWork;
        let uuid = generate_uuid(&info.szDevice);
        let display_id = available_monitors()
            .iter()
            .position(|handle| handle.0 == monitor.0)
            .unwrap();
        let scale_factor =
            get_scale_factor_for_monitor(monitor).expect("unable to get scale factor for monitor");
        let physical_size = size(
            (monitor_size.right - monitor_size.left).into(),
            (monitor_size.bottom - monitor_size.top).into(),
        );
        let physical_work_area = rect_device_bounds(monitor_work_area);

        WindowsDisplay {
            handle: monitor,
            display_id: DisplayId(display_id as _),
            scale_factor,
            bounds: Bounds {
                origin: logical_point(
                    monitor_size.left as f32,
                    monitor_size.top as f32,
                    scale_factor,
                ),
                size: physical_size.to_pixels(scale_factor),
            },
            work_area: device_bounds_to_logical(physical_work_area, scale_factor),
            physical_bounds: Bounds {
                origin: point(monitor_size.left.into(), monitor_size.top.into()),
                size: physical_size,
            },
            physical_work_area,
            uuid,
        }
    }

    fn new_with_handle_and_id(handle: HMONITOR, display_id: DisplayId) -> Self {
        let info = get_monitor_info(handle).expect("unable to get monitor info");
        let monitor_size = info.monitorInfo.rcMonitor;
        let monitor_work_area = info.monitorInfo.rcWork;
        let uuid = generate_uuid(&info.szDevice);
        let scale_factor =
            get_scale_factor_for_monitor(handle).expect("unable to get scale factor for monitor");
        let physical_size = size(
            (monitor_size.right - monitor_size.left).into(),
            (monitor_size.bottom - monitor_size.top).into(),
        );
        let physical_work_area = rect_device_bounds(monitor_work_area);

        WindowsDisplay {
            handle,
            display_id,
            scale_factor,
            bounds: Bounds {
                origin: logical_point(
                    monitor_size.left as f32,
                    monitor_size.top as f32,
                    scale_factor,
                ),
                size: physical_size.to_pixels(scale_factor),
            },
            work_area: device_bounds_to_logical(physical_work_area, scale_factor),
            physical_bounds: Bounds {
                origin: point(monitor_size.left.into(), monitor_size.top.into()),
                size: physical_size,
            },
            physical_work_area,
            uuid,
        }
    }

    pub fn primary_monitor() -> Option<Self> {
        // https://devblogs.microsoft.com/oldnewthing/20070809-00/?p=25643
        const POINT_ZERO: POINT = POINT { x: 0, y: 0 };
        let monitor = unsafe { MonitorFromPoint(POINT_ZERO, MONITOR_DEFAULTTOPRIMARY) };
        if monitor.is_invalid() {
            log::error!(
                "can not find the primary monitor: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }
        Some(WindowsDisplay::new_with_handle(monitor))
    }

    /// Check if the center point of given bounds is inside this monitor
    pub fn check_given_bounds(&self, bounds: Bounds<Pixels>) -> bool {
        let center = bounds.center();
        let center = POINT {
            x: (center.x.0 * self.scale_factor) as i32,
            y: (center.y.0 * self.scale_factor) as i32,
        };
        let monitor = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONULL) };
        if monitor.is_invalid() {
            false
        } else {
            let display = WindowsDisplay::new_with_handle(monitor);
            display.uuid == self.uuid
        }
    }

    pub fn displays() -> Vec<Rc<dyn PlatformDisplay>> {
        available_monitors()
            .into_iter()
            .enumerate()
            .map(|(id, handle)| {
                Rc::new(WindowsDisplay::new_with_handle_and_id(
                    handle,
                    DisplayId(id as _),
                )) as Rc<dyn PlatformDisplay>
            })
            .collect()
    }

    // ShellDeck patch: keep concrete monitor metrics so Windows can expose physical desktop coordinates.
    pub(crate) fn desktop_metrics() -> Vec<DesktopDisplayMetrics> {
        available_monitors()
            .into_iter()
            .enumerate()
            .map(|(id, handle)| Self::new_with_handle_and_id(handle, DisplayId(id as u32)))
            .map(|display| DesktopDisplayMetrics {
                id: display.display_id,
                global_bounds: device_bounds_to_pixels(display.physical_bounds),
                global_work_area: device_bounds_to_pixels(display.physical_work_area),
                logical_work_area: display.work_area,
                scale_factor: display.scale_factor,
            })
            .collect()
    }

    /// Check if this monitor is still online
    pub fn is_connected(hmonitor: HMONITOR) -> bool {
        available_monitors().iter().contains(&hmonitor)
    }

    pub fn physical_bounds(&self) -> Bounds<DevicePixels> {
        self.physical_bounds
    }
}

impl PlatformDisplay for WindowsDisplay {
    fn id(&self) -> DisplayId {
        self.display_id
    }

    fn uuid(&self) -> anyhow::Result<Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    // ShellDeck patch: Windows rcWork gives the true taskbar-aware per-monitor work area.
    fn work_area(&self) -> Bounds<Pixels> {
        self.work_area
    }

    // ShellDeck patch: report the monitor DPI scale alongside global display geometry.
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
}

fn rect_device_bounds(rect: RECT) -> Bounds<DevicePixels> {
    Bounds {
        origin: point(rect.left.into(), rect.top.into()),
        size: size(
            (rect.right - rect.left).into(),
            (rect.bottom - rect.top).into(),
        ),
    }
}

fn device_bounds_to_logical(
    bounds: Bounds<DevicePixels>,
    scale_factor: f32,
) -> Bounds<Pixels> {
    Bounds {
        origin: logical_point(
            bounds.origin.x.0 as f32,
            bounds.origin.y.0 as f32,
            scale_factor,
        ),
        size: bounds.size.to_pixels(scale_factor),
    }
}

fn device_bounds_to_pixels(bounds: Bounds<DevicePixels>) -> Bounds<Pixels> {
    Bounds {
        origin: point(px(bounds.origin.x.0 as f32), px(bounds.origin.y.0 as f32)),
        size: size(
            px(bounds.size.width.0 as f32),
            px(bounds.size.height.0 as f32),
        ),
    }
}

fn available_monitors() -> SmallVec<[HMONITOR; 4]> {
    let mut monitors: SmallVec<[HMONITOR; 4]> = SmallVec::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_enum_proc),
            LPARAM(&mut monitors as *mut _ as _),
        )
        .ok()
        .log_err();
    }
    monitors
}

unsafe extern "system" fn monitor_enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _place: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = data.0 as *mut SmallVec<[HMONITOR; 4]>;
    unsafe { (*monitors).push(hmonitor) };
    BOOL(1)
}

fn get_monitor_info(hmonitor: HMONITOR) -> anyhow::Result<MONITORINFOEXW> {
    let mut monitor_info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    monitor_info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let status = unsafe {
        GetMonitorInfoW(
            hmonitor,
            &mut monitor_info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if status.as_bool() {
        Ok(monitor_info)
    } else {
        Err(anyhow::anyhow!(std::io::Error::last_os_error()))
    }
}

fn generate_uuid(device_name: &[u16]) -> Uuid {
    let name = device_name
        .iter()
        .flat_map(|&a| a.to_be_bytes())
        .collect_vec();
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, &name)
}

fn get_scale_factor_for_monitor(monitor: HMONITOR) -> Result<f32> {
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }?;
    assert_eq!(dpi_x, dpi_y);
    Ok(dpi_x as f32 / USER_DEFAULT_SCREEN_DPI as f32)
}
