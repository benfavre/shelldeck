//! Shared "follow latest" behavior for streaming, vertically-scrolled views.
//!
//! GPUI exposes the last laid-out scroll range through `ScrollHandle`. Checking
//! that range before appending content lets a stream stay pinned only while the
//! reader is already at the end; scrolling up therefore remains stable.

use gpui::ScrollHandle;

const END_SLOP_PX: f64 = 2.0;

fn is_at_vertical_end(max_offset: f64, offset: f64) -> bool {
    (max_offset + offset).abs() <= END_SLOP_PX
}

/// Pin the next layout to the new bottom only when the current layout is
/// already at its bottom. Returns whether following remains active.
pub(crate) fn follow_latest_if_at_end(handle: &ScrollHandle) -> bool {
    let max_offset = handle.max_offset().height.to_f64();
    let offset = handle.offset().y.to_f64();
    let following = is_at_vertical_end(max_offset, offset);
    if following {
        handle.scroll_to_bottom();
    }
    following
}

/// Explicit navigation and new user turns intentionally resume following.
pub(crate) fn pin_to_latest(handle: &ScrollHandle) {
    handle.scroll_to_bottom();
}

#[cfg(test)]
mod tests {
    use super::is_at_vertical_end;

    // SDTEST-1687
    #[test]
    fn detects_bottom_with_small_layout_rounding_slop() {
        assert!(is_at_vertical_end(0.0, 0.0));
        assert!(is_at_vertical_end(420.0, -420.0));
        assert!(is_at_vertical_end(420.0, -418.5));
        assert!(!is_at_vertical_end(420.0, -410.0));
        assert!(!is_at_vertical_end(420.0, 0.0));
    }
}
