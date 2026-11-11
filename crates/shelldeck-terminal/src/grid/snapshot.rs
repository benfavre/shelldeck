//! Binary snapshot of a [`TerminalGrid`] for the session daemon
//! (docs/design/session-daemon.md, section 6).
//!
//! Layout: `u8 format (=1)`, then sections `u8 tag | u32 length | body`, all
//! integers big-endian. Unknown tags fail the restore. The snapshot holds
//! the screen, the modes and the saved primary screen, never the history:
//! history travels separately, newest first, as history chunks
//! ([`TerminalGrid::history_rows`] / [`TerminalGrid::prepend_history`]).
//!
//! Client-local state is not part of a snapshot and survives a restore:
//! scroll offset (clamped), `response_tx`, `max_scrollback`.

use super::{
    Cell, CellAttributes, CellWidth, Charset, CharsetSlot, CursorShape, CursorState, LineFlags,
    MouseEncoding, MouseMode, PromptMark, TerminalGrid, UnderlineStyle,
};
use crate::colors::{NamedColor, TermColor};
use smallvec::SmallVec;
use std::collections::HashMap;

pub const SNAPSHOT_FORMAT: u8 = 1;
/// Largest row or column count a snapshot may declare.
pub const MAX_SNAPSHOT_DIMENSION: usize = 1000;
/// Screen rows can exceed `rows` after a reflow (existing grid behaviour).
const MAX_SCREEN_ROWS: usize = 4 * MAX_SNAPSHOT_DIMENSION;
const MAX_LINK_BYTES: usize = 2048;
const MAX_TAIL_SECTION: usize = 64 * 1024;
const NO_LINK: u16 = 0xffff;
/// Decoded size a snapshot may expand to (screen plus saved primary).
/// Cells are run-length encoded and every cell owns a copy of its
/// hyperlink, so a few kilobytes on the wire can ask for gigabytes: the
/// decoder charges each run against this budget before it allocates.
pub const MAX_SNAPSHOT_DECODED_BYTES: usize = 256 * 1024 * 1024;
/// Decoded size one history chunk may expand to.
pub const MAX_HISTORY_DECODED_BYTES: usize = 64 * 1024 * 1024;

const TAG_SIZE: u8 = 1;
const TAG_CURSOR: u8 = 2;
const TAG_PEN: u8 = 3;
const TAG_MODES: u8 = 4;
const TAG_REGION: u8 = 5;
const TAG_CHARSETS: u8 = 6;
const TAG_SCREEN: u8 = 7;
const TAG_SAVED_PRIMARY: u8 = 8;
const TAG_META: u8 = 9;
const TAG_PROMPT: u8 = 10;
const TAG_TAIL: u8 = 11;
const TAG_LINKS: u8 = 12;
const MAX_TAG: u8 = TAG_LINKS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    Truncated,
    UnknownFormat(u8),
    UnknownSection(u8),
    DuplicateSection(u8),
    MissingSection(u8),
    TrailingBytes,
    Invalid(&'static str),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "snapshot is truncated"),
            Self::UnknownFormat(v) => write!(f, "unknown snapshot format {v}"),
            Self::UnknownSection(t) => write!(f, "unknown snapshot section {t}"),
            Self::DuplicateSection(t) => write!(f, "duplicate snapshot section {t}"),
            Self::MissingSection(t) => write!(f, "missing snapshot section {t}"),
            Self::TrailingBytes => write!(f, "trailing bytes in snapshot"),
            Self::Invalid(what) => write!(f, "invalid snapshot: {what}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

type Result<T> = std::result::Result<T, SnapshotError>;

/// One history chunk produced by [`TerminalGrid::history_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryChunk {
    /// Encoded rows, newest first.
    pub bytes: Vec<u8>,
    /// Number of rows in `bytes`.
    pub count: u16,
    /// Absolute index of the oldest row in the chunk: pass it as `end` to
    /// get the next older chunk.
    pub next_end: u64,
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

fn clamp_u16(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_bool(out: &mut Vec<u8>, value: bool) {
    out.push(u8::from(value));
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    let mut end = value.len().min(u16::MAX as usize);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    put_u16(out, end as u16);
    out.extend_from_slice(&value.as_bytes()[..end]);
}

fn put_section(out: &mut Vec<u8>, tag: u8, body: &[u8]) {
    out.push(tag);
    put_u32(out, body.len() as u32);
    out.extend_from_slice(body);
}

fn put_color(out: &mut Vec<u8>, color: TermColor) {
    match color {
        TermColor::Default => out.push(0),
        TermColor::Named(named) => out.extend_from_slice(&[1, named.to_index()]),
        TermColor::Indexed(index) => out.extend_from_slice(&[2, index]),
        TermColor::Rgb(r, g, b) => out.extend_from_slice(&[3, r, g, b]),
    }
}

struct LinkTable<'a> {
    index: HashMap<&'a str, u16>,
    urls: Vec<&'a str>,
}

impl<'a> LinkTable<'a> {
    fn new() -> Self {
        Self {
            index: HashMap::new(),
            urls: Vec::new(),
        }
    }

    fn id(&mut self, link: Option<&'a String>) -> u16 {
        let Some(url) = link else { return NO_LINK };
        if url.len() > MAX_LINK_BYTES {
            return NO_LINK;
        }
        if let Some(id) = self.index.get(url.as_str()) {
            return *id;
        }
        if self.urls.len() >= NO_LINK as usize {
            return NO_LINK;
        }
        let id = self.urls.len() as u16;
        self.urls.push(url);
        self.index.insert(url, id);
        id
    }

    fn encode(&self, out: &mut Vec<u8>) {
        put_u16(out, self.urls.len() as u16);
        for url in &self.urls {
            put_str(out, url);
        }
    }
}

fn put_attrs<'a>(out: &mut Vec<u8>, attrs: &'a CellAttributes, links: &mut LinkTable<'a>) {
    let bits = u8::from(attrs.bold)
        | u8::from(attrs.dim) << 1
        | u8::from(attrs.italic) << 2
        | u8::from(attrs.strikethrough) << 3
        | u8::from(attrs.blink) << 4
        | u8::from(attrs.inverse) << 5
        | u8::from(attrs.hidden) << 6
        | u8::from(attrs.overline) << 7;
    out.push(bits);
    out.push(match attrs.underline {
        UnderlineStyle::None => 0,
        UnderlineStyle::Single => 1,
        UnderlineStyle::Double => 2,
        UnderlineStyle::Curly => 3,
        UnderlineStyle::Dotted => 4,
        UnderlineStyle::Dashed => 5,
    });
    match attrs.underline_color {
        None => out.push(0),
        Some(color) => {
            out.push(1);
            put_color(out, color);
        }
    }
    put_u16(out, links.id(attrs.hyperlink.as_ref()));
}

fn put_cell<'a>(out: &mut Vec<u8>, cell: &'a Cell, links: &mut LinkTable<'a>) {
    put_u32(out, cell.c as u32);
    let combining = cell.combining.len().min(u8::MAX as usize);
    out.push(combining as u8);
    for c in &cell.combining[..combining] {
        put_u32(out, *c as u32);
    }
    put_color(out, cell.fg);
    put_color(out, cell.bg);
    put_attrs(out, &cell.attrs, links);
    out.push(match cell.wide {
        CellWidth::Normal => 0,
        CellWidth::Wide => 1,
        CellWidth::Spacer => 2,
    });
}

fn cell_eq(a: &Cell, b: &Cell) -> bool {
    a.c == b.c
        && a.wide == b.wide
        && a.fg == b.fg
        && a.bg == b.bg
        && a.combining == b.combining
        && a.attrs == b.attrs
}

/// Row = line flags + run-length encoded cells, normalized to `width`
/// columns (short rows are padded with `blank`, long ones cut).
fn put_row<'a>(
    out: &mut Vec<u8>,
    row: &'a [Cell],
    flags: LineFlags,
    width: usize,
    blank: &'a Cell,
    links: &mut LinkTable<'a>,
) {
    out.push(u8::from(flags.soft_wrapped));
    let cell_at = |i: usize| row.get(i).unwrap_or(blank);
    let mut i = 0;
    while i < width {
        let cell = cell_at(i);
        let mut run = 1;
        while i + run < width && cell_eq(cell, cell_at(i + run)) {
            run += 1;
        }
        put_u16(out, run as u16);
        put_cell(out, cell, links);
        i += run;
    }
}

fn put_cursor(out: &mut Vec<u8>, cursor: &CursorState) {
    put_u16(out, clamp_u16(cursor.row));
    put_u16(out, clamp_u16(cursor.col));
    put_bool(out, cursor.visible);
    out.push(match cursor.shape {
        CursorShape::Block => 0,
        CursorShape::Underline => 1,
        CursorShape::Bar => 2,
    });
    put_bool(out, cursor.blink);
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or(SnapshotError::Truncated)?;
        let bytes = self
            .buf
            .get(self.pos..end)
            .ok_or(SnapshotError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SnapshotError::Invalid("boolean")),
        }
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(b);
        Ok(u64::from_be_bytes(raw))
    }

    fn char(&mut self) -> Result<char> {
        char::from_u32(self.u32()?).ok_or(SnapshotError::Invalid("character"))
    }

    fn string(&mut self, max: usize) -> Result<String> {
        let len = self.u16()? as usize;
        if len > max {
            return Err(SnapshotError::Invalid("string length"));
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| SnapshotError::Invalid("string encoding"))
    }

    fn is_empty(&self) -> bool {
        self.pos == self.buf.len()
    }

    fn finish(self) -> Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(SnapshotError::TrailingBytes)
        }
    }

    fn color(&mut self) -> Result<TermColor> {
        Ok(match self.u8()? {
            0 => TermColor::Default,
            1 => TermColor::Named(named_from_index(self.u8()?)?),
            2 => TermColor::Indexed(self.u8()?),
            3 => {
                let b = self.take(3)?;
                TermColor::Rgb(b[0], b[1], b[2])
            }
            _ => return Err(SnapshotError::Invalid("colour")),
        })
    }

    fn link(&mut self, links: &[String]) -> Result<Option<String>> {
        match self.u16()? {
            NO_LINK => Ok(None),
            id => links
                .get(id as usize)
                .cloned()
                .map(Some)
                .ok_or(SnapshotError::Invalid("link index")),
        }
    }

    fn attrs(&mut self, links: &[String]) -> Result<CellAttributes> {
        let bits = self.u8()?;
        let underline = match self.u8()? {
            0 => UnderlineStyle::None,
            1 => UnderlineStyle::Single,
            2 => UnderlineStyle::Double,
            3 => UnderlineStyle::Curly,
            4 => UnderlineStyle::Dotted,
            5 => UnderlineStyle::Dashed,
            _ => return Err(SnapshotError::Invalid("underline style")),
        };
        let underline_color = if self.bool()? {
            Some(self.color()?)
        } else {
            None
        };
        Ok(CellAttributes {
            bold: bits & 1 != 0,
            dim: bits & 2 != 0,
            italic: bits & 4 != 0,
            strikethrough: bits & 8 != 0,
            blink: bits & 16 != 0,
            inverse: bits & 32 != 0,
            hidden: bits & 64 != 0,
            overline: bits & 128 != 0,
            underline,
            underline_color,
            hyperlink: self.link(links)?,
        })
    }

    fn cell(&mut self, links: &[String]) -> Result<Cell> {
        let c = self.char()?;
        let count = self.u8()? as usize;
        let mut combining = SmallVec::new();
        for _ in 0..count {
            combining.push(self.char()?);
        }
        let fg = self.color()?;
        let bg = self.color()?;
        let attrs = self.attrs(links)?;
        let wide = match self.u8()? {
            0 => CellWidth::Normal,
            1 => CellWidth::Wide,
            2 => CellWidth::Spacer,
            _ => return Err(SnapshotError::Invalid("cell width")),
        };
        Ok(Cell {
            c,
            combining,
            fg,
            bg,
            attrs,
            wide,
        })
    }

    /// Decode one row. Every run is charged against `budget` (bytes the
    /// decoded cells will occupy, hyperlink copies included) before its
    /// cells are allocated.
    fn row(
        &mut self,
        width: usize,
        links: &[String],
        budget: &mut usize,
    ) -> Result<(Vec<Cell>, LineFlags)> {
        let flags = LineFlags {
            soft_wrapped: self.bool()?,
        };
        let mut row = Vec::new();
        while row.len() < width {
            let run = self.u16()? as usize;
            if run == 0 || run > width - row.len() {
                return Err(SnapshotError::Invalid("cell run"));
            }
            let cell = self.cell(links)?;
            let cost = run.saturating_mul(cell_cost(&cell));
            if cost > *budget {
                return Err(SnapshotError::Invalid("decoded size"));
            }
            *budget -= cost;
            if row.is_empty() {
                row.reserve_exact(width);
            }
            for _ in 1..run {
                row.push(cell.clone());
            }
            row.push(cell);
        }
        Ok((row, flags))
    }

    /// Validate one row without keeping its cells.
    fn skip_row(&mut self, width: usize, links: &[String]) -> Result<()> {
        self.bool()?;
        let mut seen = 0;
        while seen < width {
            let run = self.u16()? as usize;
            if run == 0 || run > width - seen {
                return Err(SnapshotError::Invalid("cell run"));
            }
            self.cell(links)?;
            seen += run;
        }
        Ok(())
    }

    fn cursor(&mut self) -> Result<CursorState> {
        Ok(CursorState {
            row: self.u16()? as usize,
            col: self.u16()? as usize,
            visible: self.bool()?,
            shape: match self.u8()? {
                0 => CursorShape::Block,
                1 => CursorShape::Underline,
                2 => CursorShape::Bar,
                _ => return Err(SnapshotError::Invalid("cursor shape")),
            },
            blink: self.bool()?,
        })
    }

    fn dimension(&mut self) -> Result<usize> {
        let value = self.u16()? as usize;
        if (1..=MAX_SNAPSHOT_DIMENSION).contains(&value) {
            Ok(value)
        } else {
            Err(SnapshotError::Invalid("dimension"))
        }
    }

    fn charset(&mut self) -> Result<Charset> {
        match self.u8()? {
            0 => Ok(Charset::Ascii),
            1 => Ok(Charset::DecSpecialGraphics),
            _ => Err(SnapshotError::Invalid("charset")),
        }
    }
}

/// Bytes one decoded cell occupies: the cell itself, its own copy of the
/// hyperlink and combining characters that spill to the heap.
fn cell_cost(cell: &Cell) -> usize {
    let link = cell.attrs.hyperlink.as_ref().map_or(0, String::len);
    let combining = if cell.combining.spilled() {
        cell.combining.capacity() * std::mem::size_of::<char>()
    } else {
        0
    };
    std::mem::size_of::<Cell>() + link + combining
}

fn named_from_index(index: u8) -> Result<NamedColor> {
    Ok(match index {
        0 => NamedColor::Black,
        1 => NamedColor::Red,
        2 => NamedColor::Green,
        3 => NamedColor::Yellow,
        4 => NamedColor::Blue,
        5 => NamedColor::Magenta,
        6 => NamedColor::Cyan,
        7 => NamedColor::White,
        8 => NamedColor::BrightBlack,
        9 => NamedColor::BrightRed,
        10 => NamedColor::BrightGreen,
        11 => NamedColor::BrightYellow,
        12 => NamedColor::BrightBlue,
        13 => NamedColor::BrightMagenta,
        14 => NamedColor::BrightCyan,
        15 => NamedColor::BrightWhite,
        _ => return Err(SnapshotError::Invalid("named colour")),
    })
}

fn read_links(body: &[u8]) -> Result<Vec<String>> {
    let mut rd = Reader::new(body);
    let links = read_link_table(&mut rd)?;
    rd.finish()?;
    Ok(links)
}

fn read_link_table(rd: &mut Reader<'_>) -> Result<Vec<String>> {
    let count = rd.u16()? as usize;
    if count >= NO_LINK as usize {
        return Err(SnapshotError::Invalid("link count"));
    }
    let mut links = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        links.push(rd.string(MAX_LINK_BYTES)?);
    }
    Ok(links)
}

/// Everything a snapshot carries, fully validated before it touches a grid.
struct Decoded {
    rows: usize,
    cols: usize,
    cursor: CursorState,
    pending_wrap: bool,
    saved_cursor: Option<(usize, usize)>,
    current_attrs: CellAttributes,
    current_fg: TermColor,
    current_bg: TermColor,
    hyperlink: Option<String>,
    last_char: char,
    origin_mode: bool,
    auto_wrap: bool,
    mouse_mode: MouseMode,
    mouse_encoding: MouseEncoding,
    bracketed_paste_mode: bool,
    application_cursor_keys: bool,
    application_keypad: bool,
    focus_reporting: bool,
    synchronized_output: bool,
    scroll_top: usize,
    scroll_bottom: usize,
    tab_stops: Vec<bool>,
    charset_g0: Charset,
    charset_g1: Charset,
    active_charset: CharsetSlot,
    cells: Vec<Vec<Cell>>,
    line_flags: Vec<LineFlags>,
    saved_primary: Option<(Vec<Vec<Cell>>, CursorState, Vec<LineFlags>)>,
    title: String,
    working_directory: Option<String>,
    palette_overrides: HashMap<u8, (u8, u8, u8)>,
    prompt_mark: Option<PromptMark>,
    prompt_mark_sequence: u64,
    command_output_start: Option<i64>,
    prompt_lines: Vec<usize>,
    tail: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Result<Decoded> {
    decode_with_budget(bytes, MAX_SNAPSHOT_DECODED_BYTES)
}

fn decode_with_budget(bytes: &[u8], mut budget: usize) -> Result<Decoded> {
    let mut rd = Reader::new(bytes);
    let format = rd.u8()?;
    if format != SNAPSHOT_FORMAT {
        return Err(SnapshotError::UnknownFormat(format));
    }
    let mut sections: [Option<&[u8]>; MAX_TAG as usize + 1] = [None; MAX_TAG as usize + 1];
    while !rd.is_empty() {
        let tag = rd.u8()?;
        let len = rd.u32()? as usize;
        let body = rd.take(len)?;
        if tag == 0 || tag > MAX_TAG {
            return Err(SnapshotError::UnknownSection(tag));
        }
        if sections[tag as usize].replace(body).is_some() {
            return Err(SnapshotError::DuplicateSection(tag));
        }
    }
    let section = |tag: u8| sections[tag as usize].ok_or(SnapshotError::MissingSection(tag));

    let links = read_links(section(TAG_LINKS)?)?;

    let mut rd = Reader::new(section(TAG_SIZE)?);
    let rows = rd.dimension()?;
    let cols = rd.dimension()?;
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_SCREEN)?);
    let count = rd.u16()? as usize;
    if count < rows || count > MAX_SCREEN_ROWS {
        return Err(SnapshotError::Invalid("screen row count"));
    }
    let mut cells = Vec::with_capacity(count);
    let mut line_flags = Vec::with_capacity(count);
    for _ in 0..count {
        let (row, flags) = rd.row(cols, &links, &mut budget)?;
        cells.push(row);
        line_flags.push(flags);
    }
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_CURSOR)?);
    let cursor = rd.cursor()?;
    if cursor.row >= count || cursor.col >= cols {
        return Err(SnapshotError::Invalid("cursor position"));
    }
    let pending_wrap = rd.bool()?;
    let saved_cursor = if rd.bool()? {
        Some((rd.u16()? as usize, rd.u16()? as usize))
    } else {
        None
    };
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_PEN)?);
    let current_attrs = rd.attrs(&links)?;
    let current_fg = rd.color()?;
    let current_bg = rd.color()?;
    let hyperlink = rd.link(&links)?;
    let last_char = rd.char()?;
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_MODES)?);
    let origin_mode = rd.bool()?;
    let auto_wrap = rd.bool()?;
    let mouse_mode = match rd.u8()? {
        0 => MouseMode::None,
        1 => MouseMode::Press,
        2 => MouseMode::ButtonTracking,
        3 => MouseMode::AnyMotion,
        _ => return Err(SnapshotError::Invalid("mouse mode")),
    };
    let mouse_encoding = match rd.u8()? {
        0 => MouseEncoding::Normal,
        1 => MouseEncoding::Sgr,
        _ => return Err(SnapshotError::Invalid("mouse encoding")),
    };
    let bracketed_paste_mode = rd.bool()?;
    let application_cursor_keys = rd.bool()?;
    let application_keypad = rd.bool()?;
    let focus_reporting = rd.bool()?;
    let synchronized_output = rd.bool()?;
    let alt_active = rd.bool()?;
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_REGION)?);
    let scroll_top = rd.u16()? as usize;
    let scroll_bottom = rd.u16()? as usize;
    if scroll_top > scroll_bottom || scroll_bottom >= rows {
        return Err(SnapshotError::Invalid("scroll region"));
    }
    let stops = rd.u16()? as usize;
    if stops > MAX_SNAPSHOT_DIMENSION {
        return Err(SnapshotError::Invalid("tab stop count"));
    }
    let bits = rd.take(stops.div_ceil(8))?;
    let tab_stops = (0..stops)
        .map(|i| bits[i / 8] & (1 << (i % 8)) != 0)
        .collect();
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_CHARSETS)?);
    let charset_g0 = rd.charset()?;
    let charset_g1 = rd.charset()?;
    let active_charset = match rd.u8()? {
        0 => CharsetSlot::G0,
        1 => CharsetSlot::G1,
        _ => return Err(SnapshotError::Invalid("charset slot")),
    };
    rd.finish()?;

    let saved_primary = match (alt_active, sections[TAG_SAVED_PRIMARY as usize]) {
        (false, None) => None,
        (true, Some(body)) => {
            let mut rd = Reader::new(body);
            let saved_rows = rd.u16()? as usize;
            let saved_cols = rd.dimension()?;
            if saved_rows == 0 || saved_rows > MAX_SCREEN_ROWS {
                return Err(SnapshotError::Invalid("saved primary row count"));
            }
            let saved_cursor = rd.cursor()?;
            let mut saved_cells = Vec::with_capacity(saved_rows);
            let mut saved_flags = Vec::with_capacity(saved_rows);
            for _ in 0..saved_rows {
                let (row, flags) = rd.row(saved_cols, &links, &mut budget)?;
                saved_cells.push(row);
                saved_flags.push(flags);
            }
            rd.finish()?;
            Some((saved_cells, saved_cursor, saved_flags))
        }
        _ => return Err(SnapshotError::Invalid("saved primary presence")),
    };

    let mut rd = Reader::new(section(TAG_META)?);
    let title = rd.string(u16::MAX as usize)?;
    let working_directory = if rd.bool()? {
        Some(rd.string(u16::MAX as usize)?)
    } else {
        None
    };
    let overrides = rd.u16()? as usize;
    if overrides > 256 {
        return Err(SnapshotError::Invalid("palette override count"));
    }
    let mut palette_overrides = HashMap::with_capacity(overrides);
    for _ in 0..overrides {
        let b = rd.take(4)?;
        palette_overrides.insert(b[0], (b[1], b[2], b[3]));
    }
    rd.finish()?;

    let mut rd = Reader::new(section(TAG_PROMPT)?);
    let prompt_mark = match rd.u8()? {
        0 => None,
        1 => Some(PromptMark::PromptStart),
        2 => Some(PromptMark::CommandStart),
        3 => Some(PromptMark::CommandExecuted),
        4 => Some(PromptMark::CommandFinished(None)),
        5 => Some(PromptMark::CommandFinished(Some(rd.u32()? as i32))),
        _ => return Err(SnapshotError::Invalid("prompt mark")),
    };
    let prompt_mark_sequence = rd.u64()?;
    let command_output_start = if rd.bool()? {
        Some(rd.u64()? as i64)
    } else {
        None
    };
    let lines = rd.u32()? as usize;
    let mut prompt_lines = Vec::with_capacity(lines.min(4096));
    for _ in 0..lines {
        prompt_lines.push(rd.u32()? as usize);
    }
    rd.finish()?;

    let tail = section(TAG_TAIL)?;
    if tail.len() > MAX_TAIL_SECTION {
        return Err(SnapshotError::Invalid("tail length"));
    }

    Ok(Decoded {
        rows,
        cols,
        cursor,
        pending_wrap,
        saved_cursor,
        current_attrs,
        current_fg,
        current_bg,
        hyperlink,
        last_char,
        origin_mode,
        auto_wrap,
        mouse_mode,
        mouse_encoding,
        bracketed_paste_mode,
        application_cursor_keys,
        application_keypad,
        focus_reporting,
        synchronized_output,
        scroll_top,
        scroll_bottom,
        tab_stops,
        charset_g0,
        charset_g1,
        active_charset,
        cells,
        line_flags,
        saved_primary,
        title,
        working_directory,
        palette_overrides,
        prompt_mark,
        prompt_mark_sequence,
        command_output_start,
        prompt_lines,
        tail: tail.to_vec(),
    })
}

impl TerminalGrid {
    /// Serialize the screen, modes and saved primary screen. History is not
    /// included. Two grids in the same state produce identical bytes.
    pub fn snapshot(&self) -> Vec<u8> {
        self.snapshot_with_tail(&[])
    }

    /// Like [`snapshot`](Self::snapshot), carrying the parser bytes that are
    /// still in flight (`StreamBoundary::tail`).
    pub fn snapshot_with_tail(&self, tail: &[u8]) -> Vec<u8> {
        let blank = Cell::default();
        let mut links = LinkTable::new();
        let mut out = Vec::with_capacity(4096);
        let mut body = Vec::with_capacity(4096);
        out.push(SNAPSHOT_FORMAT);

        put_u16(&mut body, clamp_u16(self.rows));
        put_u16(&mut body, clamp_u16(self.cols));
        put_section(&mut out, TAG_SIZE, &body);

        body.clear();
        put_cursor(&mut body, &self.cursor);
        put_bool(&mut body, self.pending_wrap);
        match self.saved_cursor {
            None => body.push(0),
            Some((row, col)) => {
                body.push(1);
                put_u16(&mut body, clamp_u16(row));
                put_u16(&mut body, clamp_u16(col));
            }
        }
        put_section(&mut out, TAG_CURSOR, &body);

        body.clear();
        put_attrs(&mut body, &self.current_attrs, &mut links);
        put_color(&mut body, self.current_fg);
        put_color(&mut body, self.current_bg);
        put_u16(&mut body, links.id(self.hyperlink.as_ref()));
        put_u32(&mut body, self.last_char as u32);
        put_section(&mut out, TAG_PEN, &body);

        body.clear();
        put_bool(&mut body, self.origin_mode);
        put_bool(&mut body, self.auto_wrap);
        body.push(match self.mouse_mode {
            MouseMode::None => 0,
            MouseMode::Press => 1,
            MouseMode::ButtonTracking => 2,
            MouseMode::AnyMotion => 3,
        });
        body.push(match self.mouse_encoding {
            MouseEncoding::Normal => 0,
            MouseEncoding::Sgr => 1,
        });
        put_bool(&mut body, self.bracketed_paste_mode);
        put_bool(&mut body, self.application_cursor_keys);
        put_bool(&mut body, self.application_keypad);
        put_bool(&mut body, self.focus_reporting);
        put_bool(&mut body, self.synchronized_output);
        put_bool(&mut body, self.alt_cells.is_some());
        put_section(&mut out, TAG_MODES, &body);

        body.clear();
        put_u16(&mut body, clamp_u16(self.scroll_top));
        put_u16(&mut body, clamp_u16(self.scroll_bottom));
        let stops = self.tab_stops.len().min(MAX_SNAPSHOT_DIMENSION);
        put_u16(&mut body, stops as u16);
        let mut bits = vec![0u8; stops.div_ceil(8)];
        for (i, stop) in self.tab_stops.iter().take(stops).enumerate() {
            if *stop {
                bits[i / 8] |= 1 << (i % 8);
            }
        }
        body.extend_from_slice(&bits);
        put_section(&mut out, TAG_REGION, &body);

        body.clear();
        let charset = |c: Charset| match c {
            Charset::Ascii => 0u8,
            Charset::DecSpecialGraphics => 1,
        };
        body.push(charset(self.charset_g0));
        body.push(charset(self.charset_g1));
        body.push(match self.active_charset {
            CharsetSlot::G0 => 0,
            CharsetSlot::G1 => 1,
        });
        put_section(&mut out, TAG_CHARSETS, &body);

        body.clear();
        let count = self.cells.len().max(self.rows).min(MAX_SCREEN_ROWS);
        put_u16(&mut body, count as u16);
        for i in 0..count {
            let row = self.cells.get(i).map(Vec::as_slice).unwrap_or(&[]);
            let flags = self.line_flags.get(i).copied().unwrap_or_default();
            put_row(&mut body, row, flags, self.cols, &blank, &mut links);
        }
        put_section(&mut out, TAG_SCREEN, &body);

        if let Some(saved) = &self.alt_cells {
            body.clear();
            let saved_rows = saved.len().clamp(1, MAX_SCREEN_ROWS);
            let saved_cols = saved
                .first()
                .map(Vec::len)
                .unwrap_or(self.cols)
                .clamp(1, MAX_SNAPSHOT_DIMENSION);
            put_u16(&mut body, saved_rows as u16);
            put_u16(&mut body, saved_cols as u16);
            let default_cursor = CursorState::default();
            put_cursor(
                &mut body,
                self.alt_cursor.as_ref().unwrap_or(&default_cursor),
            );
            for i in 0..saved_rows {
                let row = saved.get(i).map(Vec::as_slice).unwrap_or(&[]);
                let flags = self
                    .alt_line_flags
                    .as_ref()
                    .and_then(|f| f.get(i).copied())
                    .unwrap_or_default();
                put_row(&mut body, row, flags, saved_cols, &blank, &mut links);
            }
            put_section(&mut out, TAG_SAVED_PRIMARY, &body);
        }

        body.clear();
        put_str(&mut body, &self.title);
        match &self.working_directory {
            None => body.push(0),
            Some(cwd) => {
                body.push(1);
                put_str(&mut body, cwd);
            }
        }
        let mut overrides: Vec<_> = self.palette_overrides.iter().collect();
        overrides.sort_unstable_by_key(|(index, _)| **index);
        put_u16(&mut body, overrides.len() as u16);
        for (index, (r, g, b)) in overrides {
            body.extend_from_slice(&[*index, *r, *g, *b]);
        }
        put_section(&mut out, TAG_META, &body);

        body.clear();
        match &self.prompt_mark {
            None => body.push(0),
            Some(PromptMark::PromptStart) => body.push(1),
            Some(PromptMark::CommandStart) => body.push(2),
            Some(PromptMark::CommandExecuted) => body.push(3),
            Some(PromptMark::CommandFinished(None)) => body.push(4),
            Some(PromptMark::CommandFinished(Some(code))) => {
                body.push(5);
                put_u32(&mut body, *code as u32);
            }
        }
        body.extend_from_slice(&self.prompt_mark_sequence.to_be_bytes());
        match self.command_output_start {
            None => body.push(0),
            Some(start) => {
                // History lengths differ between daemon and client, so the
                // row travels relative to the first screen row.
                body.push(1);
                let relative = start as i64 - self.scrollback.len() as i64;
                body.extend_from_slice(&relative.to_be_bytes());
            }
        }
        put_u32(&mut body, self.prompt_lines.len() as u32);
        for line in &self.prompt_lines {
            put_u32(&mut body, (*line).min(u32::MAX as usize) as u32);
        }
        put_section(&mut out, TAG_PROMPT, &body);

        let tail = &tail[..tail.len().min(MAX_TAIL_SECTION)];
        put_section(&mut out, TAG_TAIL, tail);

        body.clear();
        links.encode(&mut body);
        put_section(&mut out, TAG_LINKS, &body);
        out
    }

    /// Replace the screen state with a snapshot and return its parser tail,
    /// which the caller feeds to a fresh parser before the live stream.
    ///
    /// The snapshot is validated completely before anything changes. The
    /// selection and any pending clipboard request are cleared, the scroll
    /// offset is kept (clamped). `keep_history` keeps the local history (a
    /// refresh that missed no bytes); otherwise it is dropped, because rows
    /// scrolled off during the missed bytes would leave a hole.
    pub fn restore(&mut self, snapshot: &[u8], keep_history: bool) -> Result<Vec<u8>> {
        let d = decode(snapshot)?;

        if !keep_history {
            self.drop_history();
        } else {
            for i in 0..self.scrollback.len() {
                if let Some(row) = self.scrollback.get_mut(i) {
                    row.resize(d.cols, Cell::default());
                }
            }
        }
        self.selection = None;
        self.clipboard_request = None;

        self.rows = d.rows;
        self.cols = d.cols;
        self.cells = d.cells;
        self.line_flags = d.line_flags;
        self.cursor = d.cursor;
        self.pending_wrap = d.pending_wrap;
        self.saved_cursor = d.saved_cursor;
        self.current_attrs = d.current_attrs;
        self.current_fg = d.current_fg;
        self.current_bg = d.current_bg;
        self.hyperlink = d.hyperlink;
        self.last_char = d.last_char;
        self.origin_mode = d.origin_mode;
        self.auto_wrap = d.auto_wrap;
        self.mouse_mode = d.mouse_mode;
        self.mouse_encoding = d.mouse_encoding;
        self.bracketed_paste_mode = d.bracketed_paste_mode;
        self.application_cursor_keys = d.application_cursor_keys;
        self.application_keypad = d.application_keypad;
        self.focus_reporting = d.focus_reporting;
        self.synchronized_output = d.synchronized_output;
        self.scroll_top = d.scroll_top;
        self.scroll_bottom = d.scroll_bottom;
        self.tab_stops = d.tab_stops;
        self.charset_g0 = d.charset_g0;
        self.charset_g1 = d.charset_g1;
        self.active_charset = d.active_charset;
        match d.saved_primary {
            Some((cells, cursor, flags)) => {
                self.alt_cells = Some(cells);
                self.alt_cursor = Some(cursor);
                self.alt_line_flags = Some(flags);
            }
            None => {
                self.alt_cells = None;
                self.alt_cursor = None;
                self.alt_line_flags = None;
            }
        }
        self.title = d.title;
        self.working_directory = d.working_directory;
        self.palette_overrides = d.palette_overrides;
        self.prompt_mark = d.prompt_mark;
        self.prompt_mark_sequence = d.prompt_mark_sequence;
        self.command_output_start = d.command_output_start.map(|relative| {
            (self.scrollback.len() as i64)
                .saturating_add(relative)
                .max(0) as usize
        });
        self.prompt_lines = d.prompt_lines;

        self.scroll_offset = self.scroll_offset.min(self.scrollback.len());
        self.row_dirty = vec![true; self.rows];
        self.any_dirty = true;
        self.dirty = true;
        Ok(d.tail)
    }

    /// Drop the local history, keeping absolute row numbering monotonic.
    fn drop_history(&mut self) {
        self.selection_history_base = self
            .selection_history_base
            .saturating_add(self.scrollback.len());
        self.scrollback.clear();
        self.scrollback_line_flags.clear();
        self.history_epoch = self.history_epoch.wrapping_add(1);
    }

    /// Counter bumped whenever the history is cleared (ED 3, RIS, a restore
    /// that drops it). A backfill that spans a bump must stop.
    pub fn history_epoch(&self) -> u64 {
        self.history_epoch
    }

    /// Absolute index of the oldest retained history row. Rows keep their
    /// absolute index while newer rows are pushed and older ones evicted.
    pub fn history_start(&self) -> u64 {
        self.selection_history_base as u64
    }

    /// Absolute index one past the newest history row.
    pub fn history_end(&self) -> u64 {
        (self.selection_history_base + self.scrollback.len()) as u64
    }

    /// Encode up to `max_rows` history rows that are older than the absolute
    /// index `end`, newest first. `count == 0` means nothing older is
    /// retained.
    pub fn history_rows(&self, end: u64, max_rows: usize) -> HistoryChunk {
        let start_abs = self.history_start();
        let end = end.min(self.history_end());
        let available = end.saturating_sub(start_abs) as usize;
        let count = available.min(max_rows).min(u16::MAX as usize);
        let blank = Cell::default();
        let mut links = LinkTable::new();
        let mut rows = Vec::new();
        for i in 0..count {
            let logical = (end - start_abs) as usize - 1 - i;
            let row = self
                .scrollback
                .get(logical)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let flags = self
                .scrollback_line_flags
                .get(logical)
                .copied()
                .unwrap_or_default();
            put_row(&mut rows, row, flags, self.cols, &blank, &mut links);
        }
        let mut bytes = Vec::with_capacity(rows.len() + 16);
        bytes.push(SNAPSHOT_FORMAT);
        put_u16(&mut bytes, clamp_u16(self.cols));
        links.encode(&mut bytes);
        put_u16(&mut bytes, count as u16);
        bytes.extend_from_slice(&rows);
        HistoryChunk {
            bytes,
            count: count as u16,
            next_end: end - count as u64,
        }
    }

    /// Insert rows from a history chunk at the old end of the history.
    /// Returns how many rows were accepted: the grid stops accepting at its
    /// own `max_scrollback`, keeping the newest rows of the chunk.
    ///
    /// Rows produced by the live stream keep appending at the new end, so
    /// the two never collide. Selection anchors and `command_output_start`
    /// are moved so they stay on the same text.
    pub fn prepend_history(&mut self, chunk: &[u8]) -> Result<usize> {
        let mut rd = Reader::new(chunk);
        let format = rd.u8()?;
        if format != SNAPSHOT_FORMAT {
            return Err(SnapshotError::UnknownFormat(format));
        }
        let cols = rd.dimension()?;
        let links = read_link_table(&mut rd)?;
        let count = rd.u16()? as usize;
        // Rows are newest first: only the first `room` of them are kept, so
        // only those are decoded. The rest is validated without allocating.
        let room = self.max_scrollback.saturating_sub(self.scrollback.len());
        let keep = count.min(room);
        let mut budget = MAX_HISTORY_DECODED_BYTES;
        let mut incoming = Vec::with_capacity(keep.min(1024));
        for _ in 0..keep {
            incoming.push(rd.row(cols, &links, &mut budget)?);
        }
        for _ in keep..count {
            rd.skip_row(cols, &links)?;
        }
        rd.finish()?;

        let accepted = incoming.len();
        if accepted == 0 {
            return Ok(0);
        }

        let mut existing = Vec::with_capacity(self.scrollback.len());
        while let Some(row) = self.scrollback.pop() {
            let flags = self.scrollback_line_flags.pop().unwrap_or_default();
            existing.push((row, flags));
        }
        self.scrollback.clear();
        self.scrollback_line_flags.clear();
        // `incoming` and `existing` are both newest first.
        for (mut row, flags) in incoming.into_iter().rev().chain(existing.into_iter().rev()) {
            row.resize(self.cols, Cell::default());
            self.scrollback.push(row);
            self.scrollback_line_flags.push(flags);
        }

        if let Some(start) = self.command_output_start.as_mut() {
            *start = start.saturating_add(accepted);
        }
        if let Some(selection) = self.selection.as_mut() {
            selection.start.row = selection.start.row.saturating_add(accepted);
            selection.end.row = selection.end.row.saturating_add(accepted);
        }
        self.dirty = true;
        Ok(accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::TerminalProcessor;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct Emulator {
        grid: Arc<Mutex<TerminalGrid>>,
        processor: TerminalProcessor,
        parser: vte::Parser,
    }

    impl Emulator {
        fn new(rows: usize, cols: usize, scrollback: usize) -> Self {
            let grid = Arc::new(Mutex::new(TerminalGrid::with_scrollback(
                rows, cols, scrollback,
            )));
            Self {
                processor: TerminalProcessor::new(grid.clone()),
                grid,
                parser: vte::Parser::new(),
            }
        }

        fn feed(&mut self, bytes: &[u8]) {
            self.processor.process_bytes(&mut self.parser, bytes);
        }

        fn snapshot(&self) -> Vec<u8> {
            self.grid.lock().snapshot()
        }
    }

    fn history_text(grid: &TerminalGrid) -> Vec<String> {
        grid.scrollback
            .iter()
            .map(|row| {
                row.iter()
                    .map(|c| c.c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Each entry is a prefix that leaves the grid in a state the snapshot
    /// must carry, and a continuation whose result depends on that state.
    fn corpus() -> Vec<(&'static str, Vec<u8>, Vec<u8>)> {
        let b = |s: &str| s.as_bytes().to_vec();
        vec![
            (
                "sgr and underline colour",
                b("\x1b[1;3;4:3;38;2;10;20;30;48;5;200;58;2;1;2;3mstyled"),
                b(" more\x1b[0m plain"),
            ),
            (
                "wide and combining",
                b("\u{4e2d}\u{6587}e\u{301}\u{1f600}"),
                b("\u{308}x\r\n\u{4e2d}"),
            ),
            (
                "pending wrap at the right margin",
                b("\x1b[1;1H0123456789012345678901234567890123456789"),
                b("W"),
            ),
            (
                "alt screen with saved primary",
                b("primary text\r\nline two\x1b[?1049h\x1b[2;3Halt \x1b[31mred"),
                b(" still alt\x1b[?1049lback"),
            ),
            (
                "scroll region and origin mode",
                b("\x1b[2;4r\x1b[?6h\x1b[1;1Hregion"),
                b("\r\na\r\nb\r\nc\r\nd\x1b[?6l\x1b[1;1Htop"),
            ),
            (
                "tab stops",
                b("\x1b[3g\x1b[1;5H\x1bH\x1b[1;21H\x1bH\r"),
                b("\tA\tB\tC"),
            ),
            (
                "osc 8 hyperlinks",
                b("\x1b]8;;https://a.example/\x1b\\one\x1b]8;;https://b.example/\x1b\\two"),
                b("three\x1b]8;;\x1b\\four"),
            ),
            (
                "osc 133 and title and cwd",
                b("\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07out\r\n\x1b]0;my title\x07\x1b]7;file://host/tmp/x\x07"),
                b("more\r\n\x1b]133;D;3\x07"),
            ),
            (
                "dec graphics and charsets",
                b("\x1b(0\x1b)0lqqk\x0e"),
                b("xx\x0fxx\x1b(Bxx"),
            ),
            (
                "modes",
                b("\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?2026h\x1b[?7l\x1b[?25l\x1b[4 q"),
                b("0123456789012345678901234567890123456789ZZ\x1b[?7hY"),
            ),
            (
                "saved cursor, rep and palette",
                b("\x1b[3;7H\x1b7\x1b[1;1Hq\x1b]4;1;rgb:12/34/56\x07"),
                b("\x1b[5b\x1b8restored"),
            ),
        ]
    }

    // SDTEST-2958
    #[test]
    fn sdtest_2958_restored_grid_continues_exactly_like_the_source() {
        for (name, prefix, continuation) in corpus() {
            let mut source = Emulator::new(6, 40, 100);
            source.feed(&prefix);
            let snapshot = source.snapshot();

            // The target starts with another size, other content and history.
            let mut target = Emulator::new(3, 11, 100);
            target.feed(b"junk\r\njunk\r\njunk\r\njunk\r\n\x1b[?1049h\x1b[5;1r");
            let tail = target
                .grid
                .lock()
                .restore(&snapshot, false)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(tail.is_empty(), "{name}");
            assert_eq!(target.snapshot(), snapshot, "{name}: restore is not exact");

            source.feed(&continuation);
            target.feed(&continuation);
            assert_eq!(
                target.snapshot(),
                source.snapshot(),
                "{name}: grids diverged after the continuation"
            );
        }
    }

    // SDTEST-2959
    #[test]
    fn sdtest_2959_restore_keeps_scroll_offset_and_drops_selection() {
        let mut source = Emulator::new(4, 20, 100);
        source.feed(b"server screen");
        let snapshot = source.snapshot();

        let mut client = Emulator::new(4, 20, 100);
        for i in 0..30 {
            client.feed(format!("line {i}\r\n").as_bytes());
        }
        {
            let mut grid = client.grid.lock();
            grid.set_scroll_offset(7);
            grid.start_selection(0, 0);
            grid.update_selection(3, 1);
            grid.clipboard_request = Some(("c".into(), "x".into()));
            let history = grid.scrollback_len();

            // A refresh that missed nothing keeps history and scroll position.
            grid.restore(&snapshot, true).expect("restore");
            assert_eq!(grid.scrollback_len(), history);
            assert_eq!(grid.scroll_offset(), 7);
            assert!(grid.selection.is_none());
            assert!(grid.clipboard_request.is_none());

            // A resync drops history, so the offset is clamped to it.
            let epoch = grid.history_epoch();
            grid.restore(&snapshot, false).expect("restore");
            assert_eq!(grid.scrollback_len(), 0);
            assert_eq!(grid.scroll_offset(), 0);
            assert_ne!(grid.history_epoch(), epoch);
        }
    }

    // SDTEST-2960
    #[test]
    fn sdtest_2960_alt_screen_resize_keeps_saved_primary_with_its_old_size() {
        let mut source = Emulator::new(6, 40, 100);
        source.feed(b"primary one\r\nprimary two\x1b[?1049halt");
        source.grid.lock().resize(4, 30);
        source.feed(b"\x1b[2;1Hafter resize");
        let snapshot = source.snapshot();

        let mut target = Emulator::new(10, 10, 100);
        target
            .grid
            .lock()
            .restore(&snapshot, false)
            .expect("restore");
        assert_eq!(target.snapshot(), snapshot);
        for emulator in [&mut source, &mut target] {
            emulator.feed(b"\x1b[?1049l");
        }
        assert_eq!(target.snapshot(), source.snapshot());
    }

    // SDTEST-2961
    #[test]
    fn sdtest_2961_command_output_start_follows_the_clients_history_length() {
        let mut source = Emulator::new(4, 20, 100);
        for i in 0..10 {
            source.feed(format!("old {i}\r\n").as_bytes());
        }
        source.feed(b"\x1b]133;C\x07out one\r\nout two");
        let snapshot = source.snapshot();
        let expected = source.grid.lock().command_output(10, 1000);
        assert!(expected.contains("out one"));

        // Client without any history.
        let client = Emulator::new(4, 20, 100);
        client
            .grid
            .lock()
            .restore(&snapshot, false)
            .expect("restore");
        assert_eq!(client.grid.lock().command_output(10, 1000), expected);
        assert_eq!(client.snapshot(), snapshot);
    }

    // SDTEST-2962
    #[test]
    fn sdtest_2962_malformed_snapshots_are_rejected_without_touching_the_grid() {
        let mut source = Emulator::new(4, 20, 100);
        source.feed(b"\x1b]8;;https://a.example/\x07link\x1b[?1049h");
        let good = source.snapshot();

        let mut target = Emulator::new(3, 9, 100);
        target.feed(b"keep me");
        let before = target.snapshot();
        let rejects = |bytes: &[u8]| {
            let mut grid = target.grid.lock();
            let result = grid.restore(bytes, false);
            assert_eq!(
                grid.snapshot(),
                before,
                "a rejected restore changed the grid"
            );
            result.expect_err("must be rejected")
        };

        assert_eq!(rejects(&[]), SnapshotError::Truncated);
        let mut bad = good.clone();
        bad[0] = 2;
        assert_eq!(rejects(&bad), SnapshotError::UnknownFormat(2));
        // Unknown section appended.
        let mut bad = good.clone();
        bad.extend_from_slice(&[13, 0, 0, 0, 0]);
        assert_eq!(rejects(&bad), SnapshotError::UnknownSection(13));
        // Duplicate Size section.
        let mut bad = good.clone();
        bad.extend_from_slice(&[TAG_SIZE, 0, 0, 0, 4, 0, 4, 0, 20]);
        assert_eq!(rejects(&bad), SnapshotError::DuplicateSection(TAG_SIZE));
        // Every truncation fails cleanly (never panics, never applies).
        for cut in 1..good.len() {
            rejects(&good[..cut]);
        }
        // Size 0 x 20 and 2000 x 20 (the Size section is first: tag, len, rows, cols).
        for rows in [0u16, 2000] {
            let mut bad = good.clone();
            bad[6..8].copy_from_slice(&rows.to_be_bytes());
            rejects(&bad);
        }
        // Flip every byte in turn: decoding must never panic.
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xff;
            let _ = target.grid.lock().restore(&bad, false);
            target.grid.lock().restore(&before, false).expect("reset");
        }
    }

    // SDTEST-2963
    #[test]
    fn sdtest_2963_history_backfill_newest_first_rebuilds_the_server_history() {
        let mut server = Emulator::new(4, 20, 1000);
        for i in 0..50 {
            server.feed(
                format!("row {i} \x1b]8;;https://x.example/{i}\x07l\x1b]8;;\x07\r\n").as_bytes(),
            );
        }
        let snapshot = server.snapshot();
        let mut client = Emulator::new(4, 20, 1000);
        client
            .grid
            .lock()
            .restore(&snapshot, false)
            .expect("restore");

        // Live output keeps flowing on both sides while the backfill runs.
        let mut end = server.grid.lock().history_end();
        let mut step = 0;
        loop {
            let chunk = server.grid.lock().history_rows(end, 7);
            if chunk.count == 0 {
                break;
            }
            end = chunk.next_end;
            let accepted = client
                .grid
                .lock()
                .prepend_history(&chunk.bytes)
                .expect("chunk");
            assert_eq!(accepted, chunk.count as usize);
            let live = format!("live {step}\r\n");
            server.feed(live.as_bytes());
            client.feed(live.as_bytes());
            step += 1;
        }
        assert!(step > 5);
        let server_grid = server.grid.lock();
        let client_grid = client.grid.lock();
        assert_eq!(history_text(&client_grid), history_text(&server_grid));
        assert_eq!(client_grid.snapshot(), server_grid.snapshot());
        let link = |g: &TerminalGrid| g.scrollback.get(3).map(|r| r[6].attrs.hyperlink.clone());
        assert_eq!(link(&client_grid), link(&server_grid));
        assert!(link(&client_grid).flatten().is_some());
    }

    // SDTEST-2964
    #[test]
    fn sdtest_2964_prepend_history_keeps_selection_and_stops_at_the_client_limit() {
        let mut server = Emulator::new(3, 20, 1000);
        for i in 0..40 {
            server.feed(format!("row {i}\r\n").as_bytes());
        }
        let snapshot = server.snapshot();
        let mut client = Emulator::new(3, 20, 10);
        client
            .grid
            .lock()
            .restore(&snapshot, false)
            .expect("restore");
        client.feed(b"\x1b]133;C\x07picked text\r\nnext\r\n");
        server.feed(b"\x1b]133;C\x07picked text\r\nnext\r\n");

        let mut grid = client.grid.lock();
        grid.start_selection(0, 0);
        grid.update_selection(5, 0);
        grid.end_selection();
        let selected = grid.selected_text();
        let output = grid.command_output(10, 1000);
        assert!(selected.as_deref().is_some_and(|t| !t.trim().is_empty()));

        // 2 rows were pushed by the live stream; the limit of 10 leaves room
        // for 8 of the 30 rows offered.
        let before = grid.scrollback_len();
        let end = server.grid.lock().history_end() - before as u64;
        let chunk = server.grid.lock().history_rows(end, 30);
        assert_eq!(chunk.count, 30);
        let accepted = grid.prepend_history(&chunk.bytes).expect("chunk");
        assert_eq!(accepted, 10 - before);
        assert_eq!(grid.scrollback_len(), 10);
        assert_eq!(grid.selected_text(), selected);
        assert_eq!(grid.command_output(10, 1000), output);
        assert_eq!(grid.prepend_history(&chunk.bytes).expect("chunk"), 0);

        // The newest rows of the chunk were kept, in order.
        let server_grid = server.grid.lock();
        let server_history = history_text(&server_grid);
        let client_history = history_text(&grid);
        assert_eq!(
            client_history,
            server_history[server_history.len() - 10..].to_vec()
        );
    }

    // SDTEST-2965
    #[test]
    fn sdtest_2965_history_clear_bumps_the_epoch_and_ends_a_backfill() {
        let mut server = Emulator::new(3, 20, 5);
        for i in 0..4 {
            server.feed(format!("row {i}\r\n").as_bytes());
        }
        let end = server.grid.lock().history_end();
        let epoch = server.grid.lock().history_epoch();

        // Eviction: rows below the retained window are no longer offered.
        for i in 0..20 {
            server.feed(format!("new {i}\r\n").as_bytes());
        }
        assert_eq!(server.grid.lock().history_epoch(), epoch);
        assert_eq!(server.grid.lock().history_rows(end, 10).count, 0);

        server.feed(b"\x1b[3J");
        let after_ed3 = server.grid.lock().history_epoch();
        assert_ne!(after_ed3, epoch);
        server.feed(b"x\r\ny\r\nz\r\nw\r\n\x1bc");
        assert_ne!(server.grid.lock().history_epoch(), after_ed3);
        assert_eq!(server.grid.lock().scrollback_len(), 0);
    }

    /// One encoded row: a single run of `width` blank cells that all carry
    /// hyperlink `link` (`NO_LINK` for none).
    fn run_row(width: u16, link: u16) -> Vec<u8> {
        let mut row = vec![0u8];
        put_u16(&mut row, width);
        put_u32(&mut row, ' ' as u32);
        row.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        put_u16(&mut row, link);
        row.push(0);
        row
    }

    fn link_table(urls: &[&str]) -> Vec<u8> {
        let mut table = Vec::new();
        put_u16(&mut table, urls.len() as u16);
        for url in urls {
            put_str(&mut table, url);
        }
        table
    }

    // SDTEST-2997
    #[test]
    fn sdtest_2997_run_length_expansion_is_charged_before_it_is_allocated() {
        let url = "h".repeat(MAX_LINK_BYTES);

        // A real snapshot whose size, link table and screen are replaced: a
        // 1000 column screen of 4000 rows, each one run whose cells all
        // reference a 2 KiB hyperlink. About 66 KiB on the wire, more than
        // 8 GiB decoded.
        let honest = Emulator::new(24, 80, 10).snapshot();
        let mut hostile = vec![SNAPSHOT_FORMAT];
        let mut rd = Reader::new(&honest[1..]);
        while !rd.is_empty() {
            let tag = rd.u8().unwrap();
            let len = rd.u32().unwrap() as usize;
            let body = rd.take(len).unwrap();
            let replaced = match tag {
                TAG_SIZE => {
                    let mut size = Vec::new();
                    put_u16(&mut size, 1000);
                    put_u16(&mut size, 1000);
                    size
                }
                TAG_LINKS => link_table(&[&url]),
                TAG_SCREEN => {
                    let mut screen = Vec::new();
                    put_u16(&mut screen, MAX_SCREEN_ROWS as u16);
                    for _ in 0..MAX_SCREEN_ROWS {
                        screen.extend_from_slice(&run_row(1000, 0));
                    }
                    screen
                }
                _ => body.to_vec(),
            };
            put_section(&mut hostile, tag, &replaced);
        }
        assert!(hostile.len() < 80 * 1024, "{}", hostile.len());
        let mut client = Emulator::new(24, 80, 10);
        client.feed(b"still here");
        let before = client.snapshot();
        assert_eq!(
            client.grid.lock().restore(&hostile, false),
            Err(SnapshotError::Invalid("decoded size"))
        );
        assert_eq!(
            client.snapshot(),
            before,
            "a refused snapshot changes nothing"
        );

        // The same screen without the hyperlink is within the budget of a
        // smaller cap only when the cap covers its cells.
        let plain = Emulator::new(24, 80, 10).snapshot();
        let cells = 24 * 80 * std::mem::size_of::<Cell>();
        assert!(decode_with_budget(&plain, cells).is_ok());
        assert!(matches!(
            decode_with_budget(&plain, cells - 1),
            Err(SnapshotError::Invalid("decoded size"))
        ));

        // History chunk: 60 000 rows of 1000 columns in under 1 MiB.
        let chunk = |link: u16, urls: &[&str]| {
            let mut bytes = vec![SNAPSHOT_FORMAT];
            put_u16(&mut bytes, 1000);
            bytes.extend_from_slice(&link_table(urls));
            put_u16(&mut bytes, 60_000);
            for _ in 0..60_000 {
                bytes.extend_from_slice(&run_row(1000, link));
            }
            bytes
        };
        // Only the rows the grid has room for are decoded and kept.
        let client = Emulator::new(3, 20, 50);
        assert_eq!(
            client.grid.lock().prepend_history(&chunk(NO_LINK, &[])),
            Ok(50)
        );
        assert_eq!(client.grid.lock().scrollback_len(), 50);
        // With room for all of it, the hyperlink copies exceed the budget.
        let client = Emulator::new(3, 20, 100_000);
        assert_eq!(
            client.grid.lock().prepend_history(&chunk(0, &[&url])),
            Err(SnapshotError::Invalid("decoded size"))
        );
        assert_eq!(client.grid.lock().scrollback_len(), 0);
        // The rows that are skipped are still validated.
        let mut bad = chunk(NO_LINK, &[]);
        let end = bad.len();
        bad[end - 1] = 9;
        let client = Emulator::new(3, 20, 50);
        assert_eq!(
            client.grid.lock().prepend_history(&bad),
            Err(SnapshotError::Invalid("cell width"))
        );
    }
}
