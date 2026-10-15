use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read, Write};

pub type BrowserId = u64;
pub type RequestId = u64;
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TABS: usize = 8;
pub const MAX_VIEWPORT: u32 = 4096;

/// Values intentionally match CEF's EVENTFLAG_* constants.
pub mod modifiers {
    pub const CAPS_LOCK: u32 = 1 << 0;
    pub const SHIFT: u32 = 1 << 1;
    pub const CONTROL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const LEFT_MOUSE: u32 = 1 << 4;
    pub const MIDDLE_MOUSE: u32 = 1 << 5;
    pub const RIGHT_MOUSE: u32 = 1 << 6;
    pub const COMMAND: u32 = 1 << 7;
    pub const NUM_LOCK: u32 = 1 << 8;
    pub const IS_KEY_PAD: u32 = 1 << 9;
    pub const IS_LEFT: u32 = 1 << 10;
    pub const IS_RIGHT: u32 = 1 << 11;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserState {
    pub browser_id: BrowserId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum PaintKind {
    View,
    Popup,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Utf16Range {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum KeyEventKind {
    RawKeyDown,
    KeyUp,
    Char,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyEvent {
    pub kind: KeyEventKind,
    pub windows_key_code: i32,
    pub native_key_code: i32,
    pub modifiers: u32,
    pub character: Option<u16>,
    pub unmodified_character: Option<u16>,
    pub is_system_key: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ImeEvent {
    SetComposition {
        text: String,
        selection_start: i32,
        selection_end: i32,
        replacement_range: Option<Utf16Range>,
    },
    Commit {
        text: String,
        replacement_range: Option<Utf16Range>,
    },
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    Create {
        browser_id: BrowserId,
        url: String,
        width: u32,
        height: u32,
        scale_factor: f32,
    },
    Close {
        browser_id: BrowserId,
    },
    Navigate {
        browser_id: BrowserId,
        url: String,
    },
    Back {
        browser_id: BrowserId,
    },
    Forward {
        browser_id: BrowserId,
    },
    Reload {
        browser_id: BrowserId,
        ignore_cache: bool,
    },
    Stop {
        browser_id: BrowserId,
    },
    Resize {
        browser_id: BrowserId,
        width: u32,
        height: u32,
        scale_factor: f32,
    },
    MouseMove {
        browser_id: BrowserId,
        x: i32,
        y: i32,
        modifiers: u32,
        leave: bool,
    },
    MouseButton {
        browser_id: BrowserId,
        x: i32,
        y: i32,
        button: MouseButton,
        pressed: bool,
        click_count: i32,
        modifiers: u32,
    },
    MouseWheel {
        browser_id: BrowserId,
        x: i32,
        y: i32,
        delta_x: i32,
        delta_y: i32,
        modifiers: u32,
    },
    Key {
        browser_id: BrowserId,
        event: KeyEvent,
    },
    Text {
        browser_id: BrowserId,
        text: String,
    },
    Focus {
        browser_id: BrowserId,
        focused: bool,
    },
    Ime {
        browser_id: BrowserId,
        event: ImeEvent,
    },
    DevTools {
        browser_id: BrowserId,
        request_id: RequestId,
        method: String,
        params: Value,
    },
    DialogResponse {
        browser_id: BrowserId,
        dialog_id: u64,
        accept: bool,
        prompt: String,
    },
    FileChooserResponse {
        browser_id: BrowserId,
        chooser_id: u64,
        paths: Vec<String>,
    },
    DownloadResponse {
        browser_id: BrowserId,
        download_id: u64,
        path: Option<String>,
    },
    Find {
        browser_id: BrowserId,
        text: String,
        forward: bool,
        match_case: bool,
        find_next: bool,
    },
    StopFinding {
        browser_id: BrowserId,
        clear_selection: bool,
    },
    Shutdown,
}

impl Command {
    pub fn browser_id(&self) -> Option<BrowserId> {
        match self {
            Self::Create { browser_id, .. }
            | Self::Close { browser_id }
            | Self::Navigate { browser_id, .. }
            | Self::Back { browser_id }
            | Self::Forward { browser_id }
            | Self::Reload { browser_id, .. }
            | Self::Stop { browser_id }
            | Self::Resize { browser_id, .. }
            | Self::MouseMove { browser_id, .. }
            | Self::MouseButton { browser_id, .. }
            | Self::MouseWheel { browser_id, .. }
            | Self::Key { browser_id, .. }
            | Self::Text { browser_id, .. }
            | Self::Focus { browser_id, .. }
            | Self::Ime { browser_id, .. }
            | Self::DevTools { browser_id, .. }
            | Self::DialogResponse { browser_id, .. }
            | Self::FileChooserResponse { browser_id, .. }
            | Self::DownloadResponse { browser_id, .. } => Some(*browser_id),
            Self::Find { browser_id, .. } | Self::StopFinding { browser_id, .. } => {
                Some(*browser_id)
            }
            Self::Shutdown => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Event {
    Ready {
        protocol_version: u32,
        engine_version: String,
    },
    Created {
        browser_id: BrowserId,
        parent_id: Option<BrowserId>,
    },
    Closed {
        browser_id: BrowserId,
    },
    State {
        state: BrowserState,
    },
    FindResult {
        browser_id: BrowserId,
        count: i32,
        active_match: i32,
        final_update: bool,
    },
    ContextMenu {
        browser_id: BrowserId,
        x: i32,
        y: i32,
        link_url: String,
        selected_text: String,
        editable: bool,
    },
    CursorChanged {
        browser_id: BrowserId,
        cursor: String,
    },
    NewWindow {
        browser_id: BrowserId,
        url: String,
    },
    ImeRangeChanged {
        browser_id: BrowserId,
        range: Utf16Range,
        character_bounds: Vec<Rect>,
    },
    TextSelectionChanged {
        browser_id: BrowserId,
        selected_text: String,
        range: Utf16Range,
    },
    /// Raw bytes follow the JSON envelope. They are complete top-left BGRA frames.
    Paint {
        browser_id: BrowserId,
        kind: PaintKind,
        width: u32,
        height: u32,
        dirty_rects: Vec<Rect>,
    },
    Popup {
        browser_id: BrowserId,
        visible: bool,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    DevToolsResult {
        browser_id: BrowserId,
        request_id: RequestId,
        result: Value,
        error: Option<String>,
    },
    Dialog {
        browser_id: BrowserId,
        dialog_id: u64,
        kind: String,
        message: String,
        default_prompt: String,
    },
    FileChooser {
        browser_id: BrowserId,
        chooser_id: u64,
        title: String,
        accept_filters: Vec<String>,
        multiple: bool,
    },
    Download {
        browser_id: BrowserId,
        download_id: u64,
        suggested_name: String,
        url: String,
    },
    DownloadProgress {
        browser_id: BrowserId,
        download_id: u64,
        received_bytes: i64,
        total_bytes: i64,
        complete: bool,
        canceled: bool,
    },
    Error {
        browser_id: Option<BrowserId>,
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    message: T,
    payload_len: usize,
}

/// JSON control header followed by raw frame bytes. Validate bounds before allocating.
pub fn read_packet<R: Read, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
) -> io::Result<(T, Vec<u8>)> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_HEADER_BYTES {
        return Err(invalid("browser header exceeds size limit"));
    }
    let mut header = vec![0; length];
    reader.read_exact(&mut header)?;
    let envelope: Envelope<T> =
        serde_json::from_slice(&header).map_err(|e| invalid(e.to_string()))?;
    if envelope.payload_len > MAX_FRAME_BYTES {
        return Err(invalid("browser payload exceeds size limit"));
    }
    let mut payload = vec![0; envelope.payload_len];
    reader.read_exact(&mut payload)?;
    Ok((envelope.message, payload))
}

pub fn write_packet<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
    payload: &[u8],
) -> io::Result<()> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(invalid("browser payload exceeds size limit"));
    }
    let header = serde_json::to_vec(&Envelope {
        message,
        payload_len: payload.len(),
    })
    .map_err(|e| invalid(e.to_string()))?;
    if header.len() > MAX_HEADER_BYTES {
        return Err(invalid("browser header exceeds size limit"));
    }
    writer.write_all(&(header.len() as u32).to_le_bytes())?;
    writer.write_all(&header)?;
    writer.write_all(payload)?;
    writer.flush()
}

pub fn frame_byte_len(width: u32, height: u32) -> io::Result<usize> {
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4));
    match len {
        Some(len)
            if width > 0
                && height > 0
                && width <= MAX_VIEWPORT
                && height <= MAX_VIEWPORT
                && len <= MAX_FRAME_BYTES =>
        {
            Ok(len)
        }
        _ => Err(invalid("invalid browser frame dimensions")),
    }
}

/// Native pages never gain file:// or Chromium-internal privileges through the address bar or an agent.
pub fn navigation_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.len() > 16 * 1024 {
        return Err("The address exceeds the browser URL limit".into());
    }
    if input == "about:blank" {
        return Ok(input.into());
    }
    let candidate = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let parsed = url::Url::parse(&candidate)
        .map_err(|_| "Enter a valid HTTP or HTTPS address".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Only HTTP and HTTPS pages are allowed".into());
    }
    Ok(parsed.to_string())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-2530 — SDUC-557: framing survives partial reads and raw pixels without JSON/base64 corruption.
    #[test]
    fn sdtest_2530_browser_framing_and_bounds() {
        let event = Event::Paint {
            browser_id: 7,
            kind: PaintKind::View,
            width: 2,
            height: 1,
            dirty_rects: vec![],
        };
        let pixels = [0, 255, 10, 255, 32, 0, 255, 255];
        let mut wire = vec![];
        write_packet(&mut wire, &event, &pixels).unwrap();
        let (decoded, received): (Event, _) = read_packet(&mut wire.as_slice()).unwrap();
        assert!(matches!(
            decoded,
            Event::Paint {
                browser_id: 7,
                width: 2,
                ..
            }
        ));
        assert_eq!(received, pixels);
        assert!(read_packet::<_, Event>(
            &mut ((MAX_HEADER_BYTES as u32 + 1).to_le_bytes().as_slice())
        )
        .is_err());
        let bad_payload = br#"{"message":"Shutdown","payload_len":67108865}"#;
        let mut bad = (bad_payload.len() as u32).to_le_bytes().to_vec();
        bad.extend_from_slice(bad_payload);
        assert!(read_packet::<_, Command>(&mut bad.as_slice()).is_err());
        assert!(frame_byte_len(u32::MAX, 2).is_err());
    }
    // SDTEST-2531 — SDUC-557: native browser navigation rejects privileged origins and embedded credentials.
    #[test]
    fn sdtest_2531_browser_navigation_boundaries() {
        assert_eq!(
            navigation_url("example.org/path").unwrap(),
            "https://example.org/path"
        );
        assert_eq!(
            navigation_url("http://127.0.0.1:3000/").unwrap(),
            "http://127.0.0.1:3000/"
        );
        for blocked in [
            "file:///etc/passwd",
            "chrome://settings",
            "devtools://devtools",
            "https://user:pass@example.org",
        ] {
            assert!(navigation_url(blocked).is_err(), "{blocked}");
        }
    }
}
