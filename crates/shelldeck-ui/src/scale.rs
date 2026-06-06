//! Proportional UI scaling.
//!
//! The whole application UI (except the terminal grid, which has its own
//! font-size + zoom controls and pixel-accurate hit-testing) scales from a
//! single knob: the window's rem size.
//!
//! Views opt in by shadowing GPUI's `px` with [`px`] from this module:
//!
//! ```ignore
//! use gpui::*;            // brings gpui::px via glob
//! use crate::scale::px;   // explicit import shadows the glob within this file
//! ```
//!
//! [`px`] returns a [`Rems`] value relative to a 16px base, so at the default
//! rem size (16px) it renders identically to `gpui::px`. When the workspace
//! raises the rem size to `16 * scale`, every length expressed through this
//! `px` grows by `scale` — text, padding, gaps, rounding, fixed widths, all of
//! it — keeping proportions intact.
//!
//! Real-pixel call sites (window bounds, mouse-position math, the terminal
//! grid) must keep using `gpui::px` explicitly so they stay in absolute
//! device pixels.

use gpui::{rems, Rems};

/// The rem base, in pixels. GPUI's default rem size. A logical `px(v)` maps to
/// `v / REM_BASE` rems, so it renders at `v` px when the rem size is unchanged.
pub const REM_BASE: f32 = 16.0;

/// The font size (in logical px) that corresponds to a 1.0 UI scale. Matches
/// `GeneralConfig::ui_font_size`'s default.
pub const BASELINE_FONT_SIZE: f32 = 14.0;

/// Scale-aware replacement for `gpui::px`. Returns a [`Rems`] length relative
/// to [`REM_BASE`] so it tracks the window's rem size.
#[inline]
pub fn px(value: f32) -> Rems {
    rems(value / REM_BASE)
}

/// Convert an "App Font Size" setting (logical px) into a UI scale factor.
/// `BASELINE_FONT_SIZE` → 1.0.
#[inline]
pub fn scale_for_font_size(ui_font_size: f32) -> f32 {
    (ui_font_size / BASELINE_FONT_SIZE).clamp(0.6, 2.0)
}

/// The rem size (in absolute px) the window should use for a given UI scale.
#[inline]
pub fn rem_size_for_scale(scale: f32) -> f32 {
    REM_BASE * scale
}
