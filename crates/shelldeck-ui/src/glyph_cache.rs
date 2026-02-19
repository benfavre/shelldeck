use gpui::*;

// ---------------------------------------------------------------------------
// Glyph cache – resolved once per font/size, reused every frame
// ---------------------------------------------------------------------------

/// Pre-resolved glyph data for ASCII characters (codepoints 0..128).
pub struct FontGlyphs {
    /// `ascii[ch as usize]` → `Some((font_id, glyph_id))` for printable ASCII.
    /// Each character stores its own font_id to handle font fallback correctly.
    pub ascii: [Option<(FontId, GlyphId)>; 128],
}

/// Cached glyph lookup table for the four font variants (regular, bold,
/// italic, bold-italic).  Built once when the font family or size changes
/// and shared across frames via `Arc`.
pub struct GlyphCache {
    pub regular: FontGlyphs,
    pub bold: FontGlyphs,
    pub italic: FontGlyphs,
    pub bold_italic: FontGlyphs,
    pub cell_width: Pixels,
    pub cell_height: Pixels,
    /// Y-offset from the top of a cell to the glyph baseline.
    pub baseline_y: Pixels,
    pub font_size: Pixels,
    /// Base font used for non-ASCII fallback shaping.
    pub base_font: Font,
}

impl GlyphCache {
    pub fn build(text_sys: &WindowTextSystem, font_family: &str, font_size: f32) -> Self {
        let fs = px(font_size);
        let family: SharedString = font_family.to_string().into();
        let base = font(family);

        let regular = Self::resolve_variant(text_sys, &base, fs);
        let bold = Self::resolve_variant(text_sys, &base.clone().bold(), fs);
        let italic = Self::resolve_variant(text_sys, &base.clone().italic(), fs);
        let bold_italic = Self::resolve_variant(text_sys, &base.clone().bold().italic(), fs);

        // Measure cell dimensions from a reference character.
        let ref_line = text_sys.shape_line(
            "W".into(),
            fs,
            &[TextRun {
                len: 1,
                font: base.clone(),
                color: hsla(1., 1., 1., 1.),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let cell_width = ref_line.width;
        let cell_height = px(font_size * 1.4);
        let ascent = ref_line.ascent;
        let descent = ref_line.descent;
        let baseline_y = (cell_height - ascent - descent) / 2.0 + ascent;

        GlyphCache {
            regular,
            bold,
            italic,
            bold_italic,
            cell_width,
            cell_height,
            baseline_y,
            font_size: fs,
            base_font: base,
        }
    }

    /// Shape every printable ASCII character (32..=126) in a single
    /// `shape_line` call and extract the per-character `GlyphId`.
    pub fn resolve_variant(text_sys: &WindowTextSystem, f: &Font, fs: Pixels) -> FontGlyphs {
        let chars: String = (32u8..=126u8).map(|b| b as char).collect();
        let byte_len = chars.len();
        let shaped = text_sys.shape_line(
            chars.into(),
            fs,
            &[TextRun {
                len: byte_len,
                font: f.clone(),
                color: hsla(1., 1., 1., 1.),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );

        let mut ascii = [None; 128];

        for run in &shaped.runs {
            let fid = run.font_id;
            for glyph in &run.glyphs {
                // glyph.index is the byte offset into the shaped string which
                // starts at codepoint 32 (space), so the actual codepoint is
                // glyph.index + 32.
                let codepoint = glyph.index + 32;
                if codepoint < 128 {
                    ascii[codepoint] = Some((fid, glyph.id));
                }
            }
        }

        FontGlyphs { ascii }
    }

    /// O(1) lookup for an ASCII character with the right style variant.
    #[inline]
    pub fn lookup(&self, ch: char, bold: bool, italic: bool) -> Option<(FontId, GlyphId)> {
        let glyphs = match (bold, italic) {
            (true, true) => &self.bold_italic,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (false, false) => &self.regular,
        };

        let idx = ch as u32;
        if idx < 128 {
            glyphs.ascii[idx as usize]
        } else {
            None
        }
    }
}
