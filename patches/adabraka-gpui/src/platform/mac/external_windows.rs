use cocoa::base::{id, nil};
use cocoa::foundation::{NSArray, NSUInteger};
use core_foundation::{
    array::CFArrayRef, base::CFRelease, dictionary::CFDictionaryRef, string::CFStringRef,
};
use core_graphics::{
    display::{CGDirectDisplayID, CGDisplayBounds, CGGetActiveDisplayList},
    geometry::CGRect,
};
use objc::{msg_send, sel, sel_impl};

use crate::{point, px, size, Bounds, Pixels};

const K_CG_NULL_WINDOW_ID: u32 = 0;
const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kCGWindowBounds: CFStringRef;
    static kCGWindowLayer: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;

    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> bool;
}

// ShellDeck patch: enumerate visible external top-level macOS windows for desktop companion geometry.
pub(super) fn visible_external_window_bounds() -> Vec<Bounds<Pixels>> {
    unsafe {
        let window_list = CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            K_CG_NULL_WINDOW_ID,
        );
        if window_list.is_null() {
            return Vec::new();
        }

        let screen_bounds = active_display_bounds();
        let count: NSUInteger = msg_send![window_list as id, count];
        let mut result = Vec::new();

        for index in 0..count {
            let window_info: id = msg_send![window_list as id, objectAtIndex: index];
            if window_info == nil || !is_normal_window(window_info) {
                continue;
            }

            let window_bounds_value: id =
                msg_send![window_info, objectForKey: kCGWindowBounds as id];
            if window_bounds_value == nil {
                continue;
            }

            let mut rect: CGRect = std::mem::zeroed();
            if !CGRectMakeWithDictionaryRepresentation(
                window_bounds_value as CFDictionaryRef,
                &mut rect,
            ) {
                continue;
            }

            let window_bounds = Bounds {
                origin: point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
                size: size(px(rect.size.width as f32), px(rect.size.height as f32)),
            };

            if f32::from(window_bounds.size.width) <= 0.0
                || f32::from(window_bounds.size.height) <= 0.0
                || is_fullscreen(window_bounds.clone(), &screen_bounds)
            {
                continue;
            }

            result.push(window_bounds);
        }

        CFRelease(window_list as _);
        result
    }
}

unsafe fn is_normal_window(window_info: id) -> bool {
    unsafe {
        let layer_value: id = msg_send![window_info, objectForKey: kCGWindowLayer as id];
        if layer_value == nil {
            return false;
        }
        let layer: i32 = msg_send![layer_value, intValue];
        if layer != 0 {
            return false;
        }

        let owner_value: id = msg_send![window_info, objectForKey: kCGWindowOwnerPID as id];
        if owner_value == nil {
            return false;
        }
        let owner_pid: i32 = msg_send![owner_value, intValue];
        owner_pid != std::process::id() as i32
    }
}

unsafe fn active_display_bounds() -> Vec<Bounds<Pixels>> {
    unsafe {
        let mut displays: Vec<CGDirectDisplayID> = Vec::with_capacity(32);
        let mut display_count = 0;
        if CGGetActiveDisplayList(
            displays.capacity() as u32,
            displays.as_mut_ptr(),
            &mut display_count,
        ) != 0
        {
            return Vec::new();
        }
        displays.set_len(display_count as usize);

        displays
            .into_iter()
            .map(|display| {
                let rect = CGDisplayBounds(display);
                Bounds {
                    origin: point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
                    size: size(px(rect.size.width as f32), px(rect.size.height as f32)),
                }
            })
            .collect()
    }
}

fn is_fullscreen(bounds: Bounds<Pixels>, screens: &[Bounds<Pixels>]) -> bool {
    screens.iter().any(|screen| {
        f32::from(bounds.origin.x) <= f32::from(screen.origin.x)
            && f32::from(bounds.origin.y) <= f32::from(screen.origin.y)
            && f32::from(bounds.origin.x + bounds.size.width)
                >= f32::from(screen.origin.x + screen.size.width)
            && f32::from(bounds.origin.y + bounds.size.height)
                >= f32::from(screen.origin.y + screen.size.height)
    })
}
