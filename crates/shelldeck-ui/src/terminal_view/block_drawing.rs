use super::*;

// ---------------------------------------------------------------------------
// Procedural block / box-drawing character renderer
// ---------------------------------------------------------------------------

/// Try to draw a block element or box-drawing character procedurally.
/// Returns `true` if the character was handled, `false` to fall through to
/// the normal font-based renderer.
#[inline]
pub(super) fn paint_block_char(
    ch: char,
    x: Pixels,
    y: Pixels,
    cell_w: Pixels,
    cell_h: Pixels,
    color: Hsla,
    window: &mut Window,
) -> bool {
    match ch {
        // ---- Block Elements (U+2580–U+259F) ----

        // Upper half block
        '\u{2580}' => {
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h * 0.5)),
                color,
            ));
            true
        }
        // Lower 1/8 .. 7/8 blocks
        '\u{2581}' => {
            let h = cell_h * 0.125;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2582}' => {
            let h = cell_h * 0.25;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2583}' => {
            let h = cell_h * 0.375;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2584}' => {
            let h = cell_h * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2585}' => {
            let h = cell_h * 0.625;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2586}' => {
            let h = cell_h * 0.75;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        '\u{2587}' => {
            let h = cell_h * 0.875;
            window.paint_quad(fill(
                Bounds::new(point(x, y + cell_h - h), size(cell_w, h)),
                color,
            ));
            true
        }
        // Full block
        '\u{2588}' => {
            window.paint_quad(fill(Bounds::new(point(x, y), size(cell_w, cell_h)), color));
            true
        }
        // Left 7/8 .. 1/8 blocks
        '\u{2589}' => {
            let w = cell_w * 0.875;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258A}' => {
            let w = cell_w * 0.75;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258B}' => {
            let w = cell_w * 0.625;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258C}' => {
            let w = cell_w * 0.5;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258D}' => {
            let w = cell_w * 0.375;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258E}' => {
            let w = cell_w * 0.25;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }
        '\u{258F}' => {
            let w = cell_w * 0.125;
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, cell_h)), color));
            true
        }

        // Right half block
        '\u{2590}' => {
            let w = cell_w * 0.5;
            window.paint_quad(fill(Bounds::new(point(x + w, y), size(w, cell_h)), color));
            true
        }

        // Shade characters
        '\u{2591}' => {
            // Light shade (25%)
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h)),
                color.opacity(0.25),
            ));
            true
        }
        '\u{2592}' => {
            // Medium shade (50%)
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h)),
                color.opacity(0.5),
            ));
            true
        }
        '\u{2593}' => {
            // Dark shade (75%)
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(cell_w, cell_h)),
                color.opacity(0.75),
            ));
            true
        }

        // Upper 1/8 block
        '\u{2594}' => {
            let h = cell_h * 0.125;
            window.paint_quad(fill(Bounds::new(point(x, y), size(cell_w, h)), color));
            true
        }
        // Right 1/8 block
        '\u{2595}' => {
            let w = cell_w * 0.125;
            window.paint_quad(fill(
                Bounds::new(point(x + cell_w - w, y), size(w, cell_h)),
                color,
            ));
            true
        }

        // ---- Box-drawing lines (U+2500–U+257F) — most common subset ----

        // ─ Horizontal line
        '\u{2500}' | '\u{2501}' => {
            let thick = if ch == '\u{2501}' { px(2.0) } else { px(1.0) };
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            true
        }
        // │ Vertical line
        '\u{2502}' | '\u{2503}' => {
            let thick = if ch == '\u{2503}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            true
        }
        // ┌ Upper-left corner
        '\u{250C}' | '\u{250F}' => {
            let thick = if ch == '\u{250F}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(thick, cell_h - (mid_y - y))),
                color,
            ));
            true
        }
        // ┐ Upper-right corner
        '\u{2510}' | '\u{2513}' => {
            let thick = if ch == '\u{2513}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x + thick, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(thick, cell_h - (mid_y - y))),
                color,
            ));
            true
        }
        // └ Lower-left corner
        '\u{2514}' | '\u{2517}' => {
            let thick = if ch == '\u{2517}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, mid_y - y + thick)),
                color,
            ));
            true
        }
        // ┘ Lower-right corner
        '\u{2518}' | '\u{251B}' => {
            let thick = if ch == '\u{251B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x + thick, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, mid_y - y + thick)),
                color,
            ));
            true
        }
        // ├ Left tee
        '\u{251C}' | '\u{2523}' => {
            let thick = if ch == '\u{2523}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), thick)),
                color,
            ));
            true
        }
        // ┤ Right tee
        '\u{2524}' | '\u{252B}' => {
            let thick = if ch == '\u{252B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x + thick, thick)),
                color,
            ));
            true
        }
        // ┬ Top tee
        '\u{252C}' | '\u{2533}' => {
            let thick = if ch == '\u{2533}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(thick, cell_h - (mid_y - y))),
                color,
            ));
            true
        }
        // ┴ Bottom tee
        '\u{2534}' | '\u{253B}' => {
            let thick = if ch == '\u{253B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, mid_y - y + thick)),
                color,
            ));
            true
        }
        // ┼ Cross
        '\u{253C}' | '\u{254B}' => {
            let thick = if ch == '\u{254B}' { px(2.0) } else { px(1.0) };
            let mid_x = x + cell_w * 0.5 - thick * 0.5;
            let mid_y = y + cell_h * 0.5 - thick * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(cell_w, thick)),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(thick, cell_h)),
                color,
            ));
            true
        }
        // ╴ Right-end stub (light)
        '\u{2574}' => {
            let mid_x = x + cell_w * 0.5;
            let mid_y = y + cell_h * 0.5 - px(0.5);
            window.paint_quad(fill(
                Bounds::new(point(x, mid_y), size(mid_x - x, px(1.0))),
                color,
            ));
            true
        }
        // ╵ Up-end stub (light)
        '\u{2575}' => {
            let mid_x = x + cell_w * 0.5 - px(0.5);
            let mid_y = y + cell_h * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, y), size(px(1.0), mid_y - y)),
                color,
            ));
            true
        }
        // ╶ Left-end stub (light)
        '\u{2576}' => {
            let mid_x = x + cell_w * 0.5;
            let mid_y = y + cell_h * 0.5 - px(0.5);
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(cell_w - (mid_x - x), px(1.0))),
                color,
            ));
            true
        }
        // ╷ Down-end stub (light)
        '\u{2577}' => {
            let mid_x = x + cell_w * 0.5 - px(0.5);
            let mid_y = y + cell_h * 0.5;
            window.paint_quad(fill(
                Bounds::new(point(mid_x, mid_y), size(px(1.0), cell_h - (mid_y - y))),
                color,
            ));
            true
        }

        _ => false,
    }
}
