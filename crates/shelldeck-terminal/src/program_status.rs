//! OSC 7501, the Program Status Protocol.
//!
//! A program tells the terminal what it is doing (idle, working, done,
//! waiting on the user, failed) instead of the terminal guessing from the
//! screen. A report is a list of `key=value` pairs separated by colons:
//!
//! ```text
//! ESC ] 7501 ; state=working:app=cargo:progress=40 ESC \
//! ```
//!
//! Specification: <https://www.superlogical.com/rex/docs/build/program-status>.
//! The rules below follow the reference parser of Ghostty
//! (`src/terminal/osc/parsers/program_status.zig`), which Herdr and Rex use:
//! forgiving about single pairs, strict about the report as a whole.
//!
//! The terminal keeps one record per id and a report replaces its record
//! completely. `state=clear` removes a record and everything beneath it.
//! A program may ask whether the protocol is supported by sending `?`; the
//! answer is [`QUERY_REPLY`].
//!
//! Everything here is untrusted text from the program: sizes are bounded,
//! `title` and `msg` are base64 that must decode to UTF-8 without control
//! characters, and a report that breaks any limit is dropped whole.

use std::collections::BTreeMap;

/// The longest whole sequence, from `ESC ]` through the terminator.
const MAX_SEQUENCE_BYTES: usize = 4096;
/// The longest body, everything after `7501;`.
const MAX_BODY_BYTES: usize = MAX_SEQUENCE_BYTES - b"\x1b]7501;".len() - b"\x1b\\".len();
const MAX_KEY_BYTES: usize = 16;
const MAX_MSG_ENCODED_BYTES: usize = 2732;
const MAX_MSG_BYTES: usize = 2048;
const MAX_TITLE_ENCODED_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 192;
const MAX_APP_BYTES: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_ID_SEGMENT_BYTES: usize = 32;
const MAX_ID_DEPTH: usize = 8;
/// Records kept per terminal. A program that reports more is misbehaving;
/// new ids are then ignored and known ones still update.
const MAX_RECORDS: usize = 256;

/// The body of the reply to a support query: `?`, then the states and
/// kinds this parser accepts. `clear` is left out because it is not a state.
pub const QUERY_REPLY: &str =
    "?:states=idle,working,done,blocked,error:kinds=permission,question,auth";

/// What a program says it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgramState {
    /// At rest, waiting for the user's next instruction.
    Idle,
    /// Running. May carry a progress percentage.
    Working,
    /// Finished a piece of work the user has not looked at yet.
    Done,
    /// Cannot continue until the user does something.
    Blocked,
    /// Failed and stopped.
    Error,
}

impl ProgramState {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "idle" => Self::Idle,
            "working" => Self::Working,
            "done" => Self::Done,
            "blocked" => Self::Blocked,
            "error" => Self::Error,
            _ => return None,
        })
    }

    /// Which record speaks for the terminal when several exist: the one a
    /// person most needs to know about.
    pub fn urgency(self) -> u8 {
        match self {
            Self::Blocked => 4,
            Self::Error => 3,
            Self::Working => 2,
            Self::Done => 1,
            Self::Idle => 0,
        }
    }
}

/// What a blocked program needs from the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockedKind {
    /// Approval to do something.
    Permission,
    /// An answer the user has to type.
    Question,
    /// A login, token or other credential.
    Auth,
}

impl BlockedKind {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "permission" => Self::Permission,
            "question" => Self::Question,
            "auth" => Self::Auth,
            _ => return None,
        })
    }
}

/// One record, as the program last reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramStatus {
    pub state: ProgramState,
    /// Only when blocked, and only a kind this parser knows.
    pub kind: Option<BlockedKind>,
    /// 0..=100, only when working or blocked. `None` is indeterminate.
    pub progress: Option<u8>,
    /// A stable machine name (`cargo`, `claude-code`). Empty when absent.
    pub app: String,
    /// A short label for people. Empty when absent.
    pub title: String,
    /// One line for people: what it is doing or waiting for. Empty when absent.
    pub msg: String,
}

/// A parsed OSC 7501 sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `OSC 7501 ; ? ST`: is the protocol supported?
    Query,
    /// Replace the record `id` (empty: the root record, the program itself).
    Report { id: String, status: ProgramStatus },
    /// Remove the record `id` and every record beneath it. An empty id
    /// removes every record.
    Clear { id: String },
}

/// Bytes allowed in a name: an `app` value and each part of an `id`.
fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-')
}

fn is_name(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_name_byte)
}

/// Bytes allowed in any value this parser reads.
fn is_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| is_name_byte(byte) || matches!(byte, b',' | b'/' | b'='))
}

/// `key=value` pairs of a body. A pair without `=` is skipped.
fn pairs(body: &str) -> impl Iterator<Item = (&str, &str)> {
    body.split(':').filter_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        Some((key.trim_matches(|c: char| c.is_ascii_whitespace()), value))
    })
}

/// Every value of `key`, unfiltered: limits apply even to a value a later
/// pair replaces or that the value alphabet would skip.
fn raw_values<'a>(body: &'a str, key: &'a str) -> impl Iterator<Item = &'a str> {
    pairs(body)
        .filter(move |(candidate, _)| *candidate == key)
        .map(|(_, value)| value)
}

/// The last value of `key` made only of allowed bytes.
fn last_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    pairs(body)
        .filter(|(candidate, value)| *candidate == key && is_value(value))
        .map(|(_, value)| value)
        .last()
}

fn valid_id(value: &str) -> bool {
    value.len() <= MAX_ID_BYTES
        && value.split('/').count() <= MAX_ID_DEPTH
        && value
            .split('/')
            .all(|segment| segment.len() <= MAX_ID_SEGMENT_BYTES && is_name(segment))
}

/// A whole number from 0 to 100. Anything else is treated as absent.
fn parse_progress(value: &str) -> Option<u8> {
    if value.is_empty() || value.len() > 3 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u8>().ok().filter(|percent| *percent <= 100)
}

/// Standard base64, padding optional. `None` for anything else.
fn base64_decode(encoded: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => u32::from(byte - b'A'),
            b'a'..=b'z' => u32::from(byte - b'a') + 26,
            b'0'..=b'9' => u32::from(byte - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }
    let padded = encoded.ends_with('=');
    let data = encoded.trim_end_matches('=');
    let padding = encoded.len() - data.len();
    if padded && (padding > 2 || !encoded.len().is_multiple_of(4)) {
        return None;
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut output = Vec::with_capacity(data.len() * 3 / 4);
    for chunk in data.as_bytes().chunks(4) {
        let mut bits = 0u32;
        for byte in chunk {
            bits = (bits << 6) | sextet(*byte)?;
        }
        match chunk.len() {
            4 => output.extend_from_slice(&[(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]),
            3 => {
                // 18 bits carry two bytes; the last two bits must be zero.
                if bits & 0b11 != 0 {
                    return None;
                }
                output.extend_from_slice(&[(bits >> 10) as u8, (bits >> 2) as u8]);
            }
            2 => {
                // 12 bits carry one byte; the last four bits must be zero.
                if bits & 0b1111 != 0 {
                    return None;
                }
                output.push((bits >> 4) as u8);
            }
            _ => return None,
        }
    }
    Some(output)
}

/// Decode a `title` or `msg`: base64 of UTF-8 text with no control
/// character, at most `max_bytes` long. `Err` discards the whole report.
fn decode_text(encoded: &str, max_bytes: usize) -> Result<String, ()> {
    let bytes = base64_decode(encoded).ok_or(())?;
    if bytes.len() > max_bytes {
        return Err(());
    }
    let text = String::from_utf8(bytes).map_err(|_| ())?;
    if text.chars().any(char::is_control) {
        return Err(());
    }
    Ok(text)
}

/// Parse the body of an OSC 7501 sequence, everything after `7501;`.
///
/// `None` means nothing from the sequence is applied: the state is missing
/// or unknown, a key or value exceeds its limit, the id is malformed (so a
/// report meant for one record never falls back to the root, which for
/// `clear` would remove everything), or a text does not decode.
pub fn parse(body: &[u8]) -> Option<Command> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    if body == b"?" {
        return Some(Command::Query);
    }
    let body = std::str::from_utf8(body).ok()?;

    if pairs(body).any(|(key, _)| key.len() > MAX_KEY_BYTES) {
        return None;
    }
    if !raw_values(body, "id").all(valid_id) {
        return None;
    }
    for (key, limit) in [
        ("app", MAX_APP_BYTES),
        ("title", MAX_TITLE_ENCODED_BYTES),
        ("msg", MAX_MSG_ENCODED_BYTES),
    ] {
        if raw_values(body, key).any(|value| value.len() > limit) {
            return None;
        }
    }
    let title = match last_value(body, "title") {
        Some(encoded) => decode_text(encoded, MAX_TITLE_BYTES).ok()?,
        None => String::new(),
    };
    let msg = match last_value(body, "msg") {
        Some(encoded) => decode_text(encoded, MAX_MSG_BYTES).ok()?,
        None => String::new(),
    };

    let id = last_value(body, "id").unwrap_or_default().to_string();
    let state = last_value(body, "state")?;
    if state == "clear" {
        return Some(Command::Clear { id });
    }
    // An unknown state discards the report rather than guessing, so a state
    // added by a later revision never turns into something else.
    let state = ProgramState::parse(state)?;

    let kind = (state == ProgramState::Blocked)
        .then(|| last_value(body, "kind").and_then(BlockedKind::parse))
        .flatten();
    let progress = matches!(state, ProgramState::Working | ProgramState::Blocked)
        .then(|| last_value(body, "progress").and_then(parse_progress))
        .flatten();
    let app = last_value(body, "app")
        .filter(|value| is_name(value))
        .unwrap_or_default()
        .to_string();

    Some(Command::Report {
        id,
        status: ProgramStatus {
            state,
            kind,
            progress,
            app,
            title,
            msg,
        },
    })
}

/// The record that speaks for a terminal, with its app resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramHeadline {
    pub state: ProgramState,
    pub kind: Option<BlockedKind>,
    pub progress: Option<u8>,
    /// Own `app`, else the nearest ancestor's. Empty when none said.
    pub app: String,
    /// `msg`, else `title`. Empty when the program sent neither.
    pub text: String,
}

/// The records of one terminal, by id. The root record has the empty id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgramStatusRecords {
    records: BTreeMap<String, ProgramStatus>,
}

impl ProgramStatusRecords {
    /// Apply a report or a clear. A query changes nothing. Returns whether
    /// the records changed.
    pub fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Query => false,
            Command::Report { id, status } => {
                if self.records.get(&id) == Some(&status) {
                    return false;
                }
                if !self.records.contains_key(&id) && self.records.len() >= MAX_RECORDS {
                    return false;
                }
                self.records.insert(id, status);
                true
            }
            Command::Clear { id } => {
                let before = self.records.len();
                if id.is_empty() {
                    self.records.clear();
                } else {
                    let children = format!("{id}/");
                    self.records
                        .retain(|key, _| key != &id && !key.starts_with(&children));
                }
                self.records.len() != before
            }
        }
    }

    /// Remove every record: the program is gone (the shell is back at its
    /// prompt) or the terminal was fully reset. Returns whether any existed.
    pub fn clear(&mut self) -> bool {
        let had_records = !self.records.is_empty();
        self.records.clear();
        had_records
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&ProgramStatus> {
        self.records.get(id)
    }

    /// `app` of the record, else of its nearest ancestor that has one.
    fn app_of(&self, id: &str) -> &str {
        let mut current = id;
        loop {
            if let Some(record) = self.records.get(current) {
                if !record.app.is_empty() {
                    return &record.app;
                }
            }
            if current.is_empty() {
                return "";
            }
            current = current.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
    }

    /// The most urgent record (blocked, then error, working, done, idle).
    /// Between equals the root comes first, then ids in order.
    pub fn headline(&self) -> Option<ProgramHeadline> {
        let mut best: Option<(&String, &ProgramStatus)> = None;
        for (id, record) in &self.records {
            if best.is_none_or(|(_, current)| record.state.urgency() > current.state.urgency()) {
                best = Some((id, record));
            }
        }
        let (id, record) = best?;
        Some(ProgramHeadline {
            state: record.state,
            kind: record.kind,
            progress: record.progress,
            app: self.app_of(id).to_string(),
            text: if record.msg.is_empty() {
                record.title.clone()
            } else {
                record.msg.clone()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(body: &str) -> (String, ProgramStatus) {
        match parse(body.as_bytes()) {
            Some(Command::Report { id, status }) => (id, status),
            other => panic!("expected a report for {body:?}, got {other:?}"),
        }
    }

    // SDTEST-3093 / SDUC-576. The examples of the specification and of the
    // reference parser: every key, the support query and its reply.
    #[test]
    fn sdtest_3093_reports_and_the_support_query_follow_the_specification() {
        assert_eq!(parse(b"?"), Some(Command::Query));
        assert_eq!(
            QUERY_REPLY,
            "?:states=idle,working,done,blocked,error:kinds=permission,question,auth"
        );

        let (id, minimal) = report("state=idle");
        assert_eq!(id, "");
        assert_eq!(
            minimal,
            ProgramStatus {
                state: ProgramState::Idle,
                kind: None,
                progress: None,
                app: String::new(),
                title: String::new(),
                msg: String::new(),
            }
        );

        // "Plan" and "Apply 3 to add, 1 to change, 0 to destroy?"
        let (id, full) = report(
            "state=blocked:kind=permission:progress=42:id=tf/plan:app=terraform\
             :title=UGxhbg==:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/",
        );
        assert_eq!(id, "tf/plan");
        assert_eq!(full.state, ProgramState::Blocked);
        assert_eq!(full.kind, Some(BlockedKind::Permission));
        assert_eq!(full.progress, Some(42));
        assert_eq!(full.app, "terraform");
        assert_eq!(full.title, "Plan");
        assert_eq!(full.msg, "Apply 3 to add, 1 to change, 0 to destroy?");

        // Padding is optional; the last value of a repeated key wins.
        assert_eq!(report("state=done:msg=aGk").1.msg, "hi");
        assert_eq!(
            report("state=idle:state=working").1.state,
            ProgramState::Working
        );
        assert_eq!(
            parse(b"state=clear:id=deploy"),
            Some(Command::Clear {
                id: "deploy".into()
            })
        );
        assert_eq!(
            parse(b"state=clear"),
            Some(Command::Clear { id: String::new() })
        );
    }

    // SDTEST-3094 / SDUC-576. Forgiving about one pair, strict about the
    // report: an unknown or malformed pair is skipped, a broken limit, id,
    // state or text drops everything, so nothing half-applies.
    #[test]
    fn sdtest_3094_a_bad_pair_is_skipped_and_a_bad_report_is_dropped_whole() {
        // Skipped pairs.
        let (_, status) = report("state=idle:future=yes:oops:kind=permission:progress=7");
        assert_eq!(status.state, ProgramState::Idle);
        // kind only means something when blocked, progress when working or blocked.
        assert_eq!((status.kind, status.progress), (None, None));
        assert_eq!(report("state=working:progress=101").1.progress, None);
        assert_eq!(report("state=working:progress=1_0").1.progress, None);
        assert_eq!(report("state=working:progress=100").1.progress, Some(100));
        assert_eq!(report("state=blocked:kind=telepathy").1.kind, None);
        // An app that is not a plain name reads as absent, and so does a
        // value with a byte outside the value alphabet.
        assert_eq!(report("state=idle:app=a/b").1.app, "");
        assert_eq!(report("state=idle:msg=@@@@").1.msg, "");

        for dropped in [
            "",
            "state=",
            "state=bogus",
            "app=cargo",
            // A key over 16 bytes, even one this parser does not know.
            "state=idle:a_key_that_is_too_long=1",
            // An id that is not names joined by single slashes: never the root.
            "state=clear:id=a//b",
            "state=clear:id=/a",
            "state=clear:id=a b",
            "state=idle:id=a/b/c/d/e/f/g/h/i",
            // A broken limit, even on a value a later pair replaces.
            "state=idle:app=a_name_longer_than_thirty_two_bytes_x:app=ok",
            // Not base64, not UTF-8, or a control character ("a\nb").
            "state=idle:msg=A",
            "state=idle:msg=/w==",
            "state=idle:title=YQpi",
            "state=idle:msg=YQ=",
        ] {
            assert_eq!(parse(dropped.as_bytes()), None, "{dropped:?}");
        }
        let long_title = format!("state=idle:title={}", "QUFB".repeat(65));
        assert_eq!(parse(long_title.as_bytes()), None);
        let huge = format!("state=idle:x={}", "a".repeat(MAX_BODY_BYTES));
        assert_eq!(parse(huge.as_bytes()), None);
    }

    // SDTEST-3095 / SDUC-576. One record per id; a report replaces it
    // completely; clear removes a subtree; the headline is the record a
    // person most needs, with the app of its nearest ancestor.
    #[test]
    fn sdtest_3095_records_replace_clear_by_subtree_and_pick_the_urgent_headline() {
        let mut records = ProgramStatusRecords::default();
        let send = |records: &mut ProgramStatusRecords, body: &str| {
            records.apply(parse(body.as_bytes()).expect(body))
        };
        assert!(records.headline().is_none());
        assert!(send(&mut records, "state=working:app=deployer:progress=10"));
        // The same report again changes nothing.
        assert!(!send(
            &mut records,
            "state=working:app=deployer:progress=10"
        ));
        assert!(send(&mut records, "state=working:id=deploy/eu:progress=80"));
        assert!(send(
            &mut records,
            "state=blocked:kind=question:id=deploy/us:msg=UHJvbW90ZT8="
        ));

        let headline = records.headline().unwrap();
        assert_eq!(headline.state, ProgramState::Blocked);
        assert_eq!(headline.kind, Some(BlockedKind::Question));
        assert_eq!(headline.text, "Promote?");
        assert_eq!(headline.app, "deployer", "inherited from the root record");

        // A report replaces its record: the message does not linger.
        assert!(send(&mut records, "state=working:id=deploy/us"));
        assert_eq!(records.get("deploy/us").unwrap().msg, "");
        assert_eq!(records.headline().unwrap().state, ProgramState::Working);
        // Between equals the root speaks.
        assert_eq!(records.headline().unwrap().progress, Some(10));

        // Clearing `deploy` removes its children and nothing else,
        // including a sibling whose id merely starts with the same letters.
        assert!(send(&mut records, "state=done:id=deployment"));
        assert!(send(&mut records, "state=clear:id=deploy"));
        assert!(records.get("deploy/eu").is_none() && records.get("deploy/us").is_none());
        assert!(records.get("deployment").is_some() && records.get("").is_some());
        assert!(!send(&mut records, "state=clear:id=deploy"));

        assert!(send(&mut records, "state=clear"));
        assert!(records.is_empty());

        // A runaway program cannot grow the table without bound.
        for index in 0..(MAX_RECORDS + 10) {
            send(&mut records, &format!("state=idle:id=r{index}"));
        }
        assert_eq!(records.records.len(), MAX_RECORDS);
        assert!(
            send(&mut records, "state=error:id=r0"),
            "known ids still update"
        );
        assert!(records.clear());
        assert!(!records.clear());
    }
}
