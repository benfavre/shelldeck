//! Byte classifier that mirrors the state of a `vte::Parser`.
//!
//! vte 0.13 does not expose its state, so a snapshot taken while the parser
//! is inside an escape sequence, a control string or a multi-byte UTF-8
//! character could not be resumed by a fresh parser. `StreamBoundary` is fed
//! the same bytes as the parser and keeps the `tail`: a short byte string
//! that puts a fresh parser in the same state. The session daemon ships that
//! tail with every snapshot (docs/design/session-daemon.md, section 5).
//!
//! The transitions below follow `vte-0.13.1/src/table.rs` and
//! `utf8parse-0.2.2/src/types.rs` exactly, including their quirks (an ESC
//! byte inside a UTF-8 sequence is swallowed as an invalid continuation).

/// Largest tail kept verbatim. Past this the tail is replaced by a canonical
/// prefix for the current kind of sequence.
pub const MAX_TAIL_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Utf8(Utf8),
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsIgnore,
    DcsPassthrough,
    SosPmApc,
    Osc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Utf8 {
    Tail3,
    Tail2,
    Tail1,
    E0,
    Ed,
    F0,
    F4,
}

impl Utf8 {
    fn start(byte: u8) -> Option<Self> {
        Some(match byte {
            0xc2..=0xdf => Self::Tail1,
            0xe0 => Self::E0,
            0xe1..=0xec | 0xee..=0xef => Self::Tail2,
            0xed => Self::Ed,
            0xf0 => Self::F0,
            0xf1..=0xf3 => Self::Tail3,
            0xf4 => Self::F4,
            _ => return None,
        })
    }

    /// Next state, or `None` when the character is complete or invalid (the
    /// parser is back at ground either way and the byte is consumed).
    fn advance(self, byte: u8) -> Option<Self> {
        match (self, byte) {
            (Self::E0, 0xa0..=0xbf) | (Self::Ed, 0x80..=0x9f) | (Self::Tail2, 0x80..=0xbf) => {
                Some(Self::Tail1)
            }
            (Self::F0, 0x90..=0xbf) | (Self::F4, 0x80..=0x8f) | (Self::Tail3, 0x80..=0xbf) => {
                Some(Self::Tail2)
            }
            _ => None,
        }
    }
}

/// Coarse classification of the parser state, for logs and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    Ground,
    Utf8,
    Escape,
    Csi,
    Dcs,
    SosPmApc,
    Osc,
}

#[derive(Debug, Clone)]
pub struct StreamBoundary {
    state: State,
    tail: Vec<u8>,
    /// The tail was replaced by a canonical prefix: stop recording until the
    /// parser is back at ground.
    truncated: bool,
}

impl Default for StreamBoundary {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamBoundary {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            tail: Vec::new(),
            truncated: false,
        }
    }

    pub fn is_ground(&self) -> bool {
        self.state == State::Ground
    }

    pub fn kind(&self) -> BoundaryKind {
        match self.state {
            State::Ground => BoundaryKind::Ground,
            State::Utf8(_) => BoundaryKind::Utf8,
            State::Escape | State::EscapeIntermediate => BoundaryKind::Escape,
            State::CsiEntry | State::CsiParam | State::CsiIntermediate | State::CsiIgnore => {
                BoundaryKind::Csi
            }
            State::DcsEntry
            | State::DcsParam
            | State::DcsIntermediate
            | State::DcsIgnore
            | State::DcsPassthrough => BoundaryKind::Dcs,
            State::SosPmApc => BoundaryKind::SosPmApc,
            State::Osc => BoundaryKind::Osc,
        }
    }

    /// Bytes that put a fresh parser in the state the mirrored parser is in.
    /// Empty at ground.
    pub fn tail(&self) -> &[u8] {
        if !self.truncated {
            return &self.tail;
        }
        // Canonical prefixes: each leaves a fresh parser in a state that ends
        // at the same byte as the mirrored one. Only the effect of the one
        // oversize sequence is lost on the client that attaches now.
        match self.state {
            State::CsiEntry | State::CsiParam | State::CsiIntermediate | State::CsiIgnore => {
                b"\x1b[0<"
            }
            State::DcsEntry => b"\x1bP",
            State::DcsParam => b"\x1bP0",
            State::DcsIntermediate => b"\x1bP ",
            State::DcsIgnore => b"\x1bP0<",
            State::DcsPassthrough => b"\x1bPq",
            State::SosPmApc => b"\x1bX",
            State::Osc => b"\x1b]7999;",
            State::Ground | State::Utf8(_) | State::Escape | State::EscapeIntermediate => {
                &self.tail
            }
        }
    }

    /// Whether the tail is a canonical prefix instead of the original bytes.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Feed the bytes that were (or are about to be) fed to the parser.
    pub fn advance(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            // Fast path: printable ASCII and plain C0 controls at ground.
            if self.state == State::Ground && byte < 0x80 && byte != 0x1b {
                continue;
            }
            self.step(byte);
        }
    }

    fn enter(&mut self, state: State, tail: &[u8]) {
        self.state = state;
        self.tail.clear();
        self.tail.extend_from_slice(tail);
        self.truncated = false;
    }

    fn ground(&mut self) {
        self.enter(State::Ground, &[]);
    }

    fn go(&mut self, state: State, byte: u8) {
        self.state = state;
        self.push(byte);
    }

    fn push(&mut self, byte: u8) {
        if self.truncated {
            return;
        }
        if self.tail.len() < MAX_TAIL_BYTES {
            self.tail.push(byte);
        } else {
            // Too long to carry: `tail()` switches to a canonical prefix.
            self.tail.clear();
            self.truncated = true;
        }
    }

    fn step(&mut self, byte: u8) {
        if let State::Utf8(utf8) = self.state {
            // Out of band in vte: no "anywhere" transitions here.
            match utf8.advance(byte) {
                Some(next) => self.go(State::Utf8(next), byte),
                None => self.ground(),
            }
            return;
        }

        // "Anywhere" transitions win over the per-state table.
        match byte {
            0x18 | 0x1a => return self.ground(),
            0x1b => return self.enter(State::Escape, b"\x1b"),
            _ => {}
        }

        // Bytes that change nothing in the parser (executed C0 controls,
        // ignored bytes, bytes absent from the table) are not recorded: C0
        // controls already took effect on the grid.
        match self.state {
            State::Ground => {
                if let Some(utf8) = Utf8::start(byte) {
                    self.enter(State::Utf8(utf8), &[byte]);
                }
            }
            State::Utf8(_) => {}
            State::Escape => match byte {
                0x20..=0x2f => self.go(State::EscapeIntermediate, byte),
                0x5b => self.go(State::CsiEntry, byte),
                0x5d => self.go(State::Osc, byte),
                0x50 => self.go(State::DcsEntry, byte),
                0x58 | 0x5e | 0x5f => self.go(State::SosPmApc, byte),
                0x30..=0x7e => self.ground(),
                _ => {}
            },
            State::EscapeIntermediate => match byte {
                // vte keeps two intermediates and sets `ignoring` on the
                // third, so ESC plus three intermediates is an exact tail.
                0x20..=0x2f if self.tail.len() < 4 => self.tail.push(byte),
                0x30..=0x7e => self.ground(),
                _ => {}
            },
            State::CsiEntry => match byte {
                0x20..=0x2f => self.go(State::CsiIntermediate, byte),
                0x30..=0x3f => self.go(State::CsiParam, byte),
                0x40..=0x7e => self.ground(),
                _ => {}
            },
            State::CsiParam => match byte {
                0x30..=0x3b => self.push(byte),
                0x3c..=0x3f => self.enter_csi_ignore(),
                0x20..=0x2f => self.go(State::CsiIntermediate, byte),
                0x40..=0x7e => self.ground(),
                _ => {}
            },
            State::CsiIntermediate => match byte {
                0x20..=0x2f => self.push(byte),
                0x30..=0x3f => self.enter_csi_ignore(),
                0x40..=0x7e => self.ground(),
                _ => {}
            },
            State::CsiIgnore => {
                if let 0x40..=0x7e = byte {
                    self.ground();
                }
            }
            State::DcsEntry => match byte {
                0x20..=0x2f => self.go(State::DcsIntermediate, byte),
                0x30..=0x3f => self.go(State::DcsParam, byte),
                0x40..=0x7e => self.go(State::DcsPassthrough, byte),
                _ => {}
            },
            State::DcsParam => match byte {
                0x30..=0x3b => self.push(byte),
                0x3c..=0x3f => self.enter_dcs_ignore(),
                0x20..=0x2f => self.go(State::DcsIntermediate, byte),
                0x40..=0x7e => self.go(State::DcsPassthrough, byte),
                _ => {}
            },
            State::DcsIntermediate => match byte {
                0x20..=0x2f => self.push(byte),
                0x30..=0x3f => self.enter_dcs_ignore(),
                0x40..=0x7e => self.go(State::DcsPassthrough, byte),
                _ => {}
            },
            State::DcsIgnore | State::SosPmApc => {
                if byte == 0x9c {
                    self.ground();
                }
            }
            State::DcsPassthrough => match byte {
                0x9c => self.ground(),
                // Data bytes are passed to the performer (`put`).
                0x00..=0x7e => self.push(byte),
                _ => {}
            },
            State::Osc => match byte {
                0x07 => self.ground(),
                0x20..=0xff => self.push(byte),
                _ => {}
            },
        }
    }

    fn enter_csi_ignore(&mut self) {
        self.state = State::CsiIgnore;
        self.tail.clear();
        self.truncated = true;
    }

    fn enter_dcs_ignore(&mut self) {
        self.state = State::DcsIgnore;
        self.tail.clear();
        self.truncated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::TerminalGrid;
    use crate::parser::TerminalProcessor;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct Emulator {
        grid: Arc<Mutex<TerminalGrid>>,
        processor: TerminalProcessor,
        parser: vte::Parser,
    }

    impl Emulator {
        fn new(rows: usize, cols: usize) -> Self {
            let grid = Arc::new(Mutex::new(TerminalGrid::new(rows, cols)));
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

    /// Split `stream` at every byte index. At each split: snapshot the
    /// "daemon" emulator with its boundary tail, restore into a fresh
    /// emulator, feed the tail then the rest. The result must equal the
    /// unsplit reference.
    fn assert_every_split_resumes(stream: &[u8]) {
        let mut reference = Emulator::new(6, 20);
        reference.feed(stream);
        let expected = reference.snapshot();

        for split in 0..=stream.len() {
            let mut daemon = Emulator::new(6, 20);
            let mut boundary = StreamBoundary::new();
            daemon.feed(&stream[..split]);
            boundary.advance(&stream[..split]);
            let snapshot = daemon.grid.lock().snapshot_with_tail(boundary.tail());

            let mut client = Emulator::new(3, 7);
            let tail = client
                .grid
                .lock()
                .restore(&snapshot, false)
                .expect("snapshot restores");
            assert_eq!(tail, boundary.tail());
            client.feed(&tail);
            client.feed(&stream[split..]);
            assert_eq!(
                client.snapshot(),
                expected,
                "split at byte {split} of {} diverged (tail {:?})",
                stream.len(),
                boundary.tail()
            );
        }
    }

    // SDTEST-2955
    #[test]
    fn sdtest_2955_attach_at_any_byte_resumes_utf8_csi_osc_and_dcs() {
        let mut stream = Vec::new();
        stream.extend_from_slice("ab\u{e9}\u{4e2d}\u{1f600}c\r\n".as_bytes());
        // CSI with an embedded C0 control (executed in the middle).
        stream.extend_from_slice(b"\x1b[1;3\n1mred\x1b[0m\x1b[2;5H");
        // Private CSI, CSI with intermediates, ignored CSI.
        stream.extend_from_slice(b"\x1b[?25l\x1b[?25h\x1b[2 q\x1b[1<2mX");
        // OSC ended by BEL and by ST, with UTF-8 payload.
        stream.extend_from_slice("\x1b]0;titre \u{e9}\x07".as_bytes());
        stream.extend_from_slice(b"\x1b]8;;https://example.org/\x1b\\link\x1b]8;;\x1b\\");
        // DCS and APC strings, one cancelled by CAN.
        stream.extend_from_slice(b"\x1bPq#0;2;0;0;0\x1b\\\x1b_apc\x9c\x1bP1$r\x18Y");
        // Charset designation, DEC graphics, save and restore cursor.
        stream.extend_from_slice(b"\x1b(0lqk\x1b(B\x1b7\x1b[3;1Hz\x1b8");
        // Invalid UTF-8, including an ESC swallowed inside a sequence.
        stream.extend_from_slice(b"\xe4\xb8\x1b[31m!\xff\xc3(\r\n");
        // Wide character at the right margin and pending wrap.
        stream.extend_from_slice("\x1b[5;19H\u{4e2d}\u{4e2d}w".as_bytes());
        assert_every_split_resumes(&stream);
    }

    // SDTEST-2956
    #[test]
    fn sdtest_2956_unterminated_strings_have_a_tail_and_never_block() {
        for (intro, kind) in [
            (&b"\x1bP"[..], BoundaryKind::Dcs),
            (&b"\x1b]"[..], BoundaryKind::Osc),
            (&b"\x1b_"[..], BoundaryKind::SosPmApc),
        ] {
            let mut boundary = StreamBoundary::new();
            boundary.advance(b"text");
            boundary.advance(intro);
            assert_eq!(boundary.kind(), kind);
            assert_eq!(boundary.tail(), intro);
            assert!(!boundary.is_truncated());
        }
    }

    // SDTEST-2957
    #[test]
    fn sdtest_2957_oversize_strings_use_a_canonical_prefix_and_end_at_the_same_byte() {
        // (introducer, filler byte, terminator, expected canonical prefix)
        type Case = (&'static [u8], u8, &'static [u8], &'static [u8]);
        let cases: [Case; 4] = [
            (b"\x1b]2;", b'x', b"\x07", b"\x1b]7999;"),
            (b"\x1bPq", b'x', b"\x9c", b"\x1bPq"),
            (b"\x1bP", b'1', b"q\x1b\\", b"\x1bP0"),
            (b"\x1b[", b'1', b"m", b"\x1b[0<"),
        ];
        for (intro, filler, end, prefix) in cases {
            let mut stream = b"before".to_vec();
            stream.extend_from_slice(intro);
            stream.extend(std::iter::repeat_n(filler, MAX_TAIL_BYTES + 100));
            let cut = stream.len();
            stream.extend_from_slice(end);
            stream.extend_from_slice(b"AFTER");

            let mut reference = Emulator::new(4, 30);
            reference.feed(&stream);

            let mut daemon = Emulator::new(4, 30);
            let mut boundary = StreamBoundary::new();
            daemon.feed(&stream[..cut]);
            boundary.advance(&stream[..cut]);
            assert!(boundary.is_truncated());
            assert_eq!(boundary.tail(), prefix);

            let snapshot = daemon.grid.lock().snapshot_with_tail(boundary.tail());
            let mut client = Emulator::new(4, 30);
            let tail = client
                .grid
                .lock()
                .restore(&snapshot, false)
                .expect("restore");
            client.feed(&tail);
            client.feed(&stream[cut..]);
            // Same text on screen: the string ended at the same byte.
            let text = |e: &Emulator| -> String {
                e.grid.lock().cells[0]
                    .iter()
                    .map(|c| c.c)
                    .collect::<String>()
            };
            assert_eq!(text(&client), text(&reference));
            assert!(text(&client).starts_with("beforeAFTER"));

            boundary.advance(&stream[cut..]);
            assert!(boundary.is_ground());
            assert!(boundary.tail().is_empty());
        }
    }
}
