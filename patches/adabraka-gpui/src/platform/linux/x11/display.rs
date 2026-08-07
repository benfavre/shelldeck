use anyhow::Context as _;
use uuid::Uuid;
use x11rb::{
    connection::Connection as _,
    protocol::xproto::{AtomEnum, ConnectionExt as _},
    xcb_ffi::XCBConnection,
};

use crate::{Bounds, DisplayId, Pixels, PlatformDisplay, Size, point, px};

#[derive(Debug)]
pub(crate) struct X11Display {
    x_screen_index: usize,
    bounds: Bounds<Pixels>,
    work_area: Bounds<Pixels>,
    scale_factor: f32,
    uuid: Uuid,
}

impl X11Display {
    pub(crate) fn new(
        xcb: &XCBConnection,
        scale_factor: f32,
        x_screen_index: usize,
    ) -> anyhow::Result<Self> {
        let screen = xcb
            .setup()
            .roots
            .get(x_screen_index)
            .with_context(|| format!("No screen found with index {x_screen_index}"))?;
        let bounds = Bounds {
            origin: Default::default(),
            size: Size {
                width: px(screen.width_in_pixels as f32 / scale_factor),
                height: px(screen.height_in_pixels as f32 / scale_factor),
            },
        };
        let work_area = ewmh_work_area(xcb, screen.root, scale_factor)
            .map(|work_area| work_area.intersect(&bounds))
            .unwrap_or(bounds);

        Ok(Self {
            x_screen_index,
            bounds,
            work_area,
            scale_factor,
            uuid: Uuid::from_bytes([0; 16]),
        })
    }
}

impl PlatformDisplay for X11Display {
    fn id(&self) -> DisplayId {
        DisplayId(self.x_screen_index as u32)
    }

    fn uuid(&self) -> anyhow::Result<Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    // ShellDeck patch: use EWMH _NET_WORKAREA when the window manager exposes it.
    fn work_area(&self) -> Bounds<Pixels> {
        self.work_area
    }

    // ShellDeck patch: report X11 scale alongside display metrics.
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
}

// ShellDeck patch: read the first EWMH work area entry and keep it root-bounds-relative.
fn ewmh_work_area(
    xcb: &XCBConnection,
    root: x11rb::protocol::xproto::Window,
    scale_factor: f32,
) -> Option<Bounds<Pixels>> {
    let atom = xcb
        .intern_atom(false, b"_NET_WORKAREA")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = xcb
        .get_property(false, root, atom, AtomEnum::CARDINAL, 0, 4)
        .ok()?
        .reply()
        .ok()?;
    let mut values = reply.value32()?;
    let x = values.next()? as f32 / scale_factor;
    let y = values.next()? as f32 / scale_factor;
    let width = values.next()? as f32 / scale_factor;
    let height = values.next()? as f32 / scale_factor;
    (width > 0.0 && height > 0.0).then_some(Bounds {
        origin: point(px(x), px(y)),
        size: Size {
            width: px(width),
            height: px(height),
        },
    })
}
