//! CEF's macOS delegate constructs NSEvent from native_key_code + characters;
//! windows_key_code alone is ignored, and two zero characters mean FlagsChanged.
//! Values match Chromium's Mac key conversion and AppKit function-key characters:
//! https://github.com/chromium/chromium/blob/main/ui/events/keycodes/keyboard_code_conversion_mac.mm
use crate::protocol::{modifiers, KeyEvent, KeyEventKind};

pub(super) fn macos_event(mut event: KeyEvent) -> KeyEvent {
    // CHAR uses its code point as windows_key_code, unlike RAWKEYDOWN's VKEY.
    let logical_key = if matches!(event.kind, KeyEventKind::Char) {
        event
            .character
            .filter(|character| (*character as u8).is_ascii_alphabetic() && *character < 128)
            .map(|character| (character as u8).to_ascii_uppercase() as i32)
            .unwrap_or(event.windows_key_code)
    } else {
        event.windows_key_code
    };
    if let Some((native, character)) = cocoa_key(logical_key) {
        if event.native_key_code == 0 {
            event.native_key_code = native;
        }
        if character != 0 {
            if event.unmodified_character.unwrap_or(0) == 0 {
                event.unmodified_character = Some(character);
            }
            if event.character.unwrap_or(0) == 0 {
                let character = if event.modifiers & modifiers::SHIFT != 0
                    && (b'a' as u16..=b'z' as u16).contains(&character)
                {
                    character - 32
                } else {
                    character
                };
                event.character = Some(character);
            }
        }
    }
    event
}
fn cocoa_key(key: i32) -> Option<(i32, u16)> {
    // Stable Apple virtual key locations; letters are indexed by Win32 VKEY_A..Z.
    const LETTERS: [i32; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    const DIGITS: [i32; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    Some(match key {
        65..=90 => (LETTERS[(key - 65) as usize], (key + 32) as u16),
        48..=57 => (DIGITS[(key - 48) as usize], key as u16),
        8 => (51, 0x7f),
        9 => (48, 9),
        13 => (36, 13),
        16 => (56, 0),
        17 => (59, 0),
        18 => (58, 0),
        20 => (57, 0),
        27 => (53, 27),
        32 => (49, 32),
        33 => (116, 0xf72c),
        34 => (121, 0xf72d),
        35 => (119, 0xf72b),
        36 => (115, 0xf729),
        37 => (123, 0xf702),
        38 => (126, 0xf700),
        39 => (124, 0xf703),
        40 => (125, 0xf701),
        45 => (114, 0xf727),
        46 => (117, 0xf728),
        91 => (55, 0),
        92 | 93 => (54, 0),
        112..=123 => (
            [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111][(key - 112) as usize],
            0xf704 + (key - 112) as u16,
        ),
        186 => (41, b';' as u16),
        187 => (24, b'=' as u16),
        188 => (43, b',' as u16),
        189 => (27, b'-' as u16),
        190 => (47, b'.' as u16),
        191 => (44, b'/' as u16),
        192 => (50, b'`' as u16),
        219 => (33, b'[' as u16),
        220 => (42, b'\\' as u16),
        221 => (30, b']' as u16),
        222 => (39, b'\'' as u16),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::KeyEventKind;
    fn raw(key: i32) -> KeyEvent {
        KeyEvent {
            kind: KeyEventKind::RawKeyDown,
            windows_key_code: key,
            native_key_code: 0,
            modifiers: 0,
            character: None,
            unmodified_character: None,
            is_system_key: false,
        }
    }
    // SDTEST-2556 — SDUC-557: native macOS keys must be real NSEvent keys, not FlagsChanged/A.
    #[test]
    fn sdtest_2556_macos_portable_keys_produce_native_events() {
        let arrow = macos_event(raw(37));
        assert_eq!(
            (
                arrow.native_key_code,
                arrow.character,
                arrow.unmodified_character
            ),
            (123, Some(0xf702), Some(0xf702))
        );
        let mut character = raw(b'b' as i32);
        character.kind = KeyEventKind::Char;
        character.character = Some(b'b' as u16);
        character.unmodified_character = Some(b'b' as u16);
        let character = macos_event(character);
        assert_eq!(
            (character.native_key_code, character.character),
            (11, Some(b'b' as u16))
        );
        let enter = macos_event(raw(13));
        assert_eq!((enter.native_key_code, enter.character), (36, Some(13)));
        let mut shortcut = raw(65);
        shortcut.modifiers = modifiers::COMMAND | modifiers::SHIFT;
        let shortcut = macos_event(shortcut);
        assert_eq!(
            (
                shortcut.native_key_code,
                shortcut.character,
                shortcut.unmodified_character
            ),
            (0, Some(b'A' as u16), Some(b'a' as u16))
        );
        let mut modifier = raw(16);
        modifier.modifiers = modifiers::SHIFT;
        let modifier = macos_event(modifier);
        assert_eq!((modifier.native_key_code, modifier.character), (56, None));
        let mut native = raw(65);
        native.native_key_code = 12;
        native.character = Some('é' as u16);
        native.unmodified_character = Some('é' as u16);
        let native = macos_event(native);
        assert_eq!(
            (native.native_key_code, native.character),
            (12, Some('é' as u16))
        );
    }
}
