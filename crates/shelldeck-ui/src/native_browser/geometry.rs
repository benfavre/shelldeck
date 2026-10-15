//! Coordinate and input conversion, independent of the GPUI renderer.
use gpui::{Bounds, Keystroke, Modifiers, Pixels, Point};
use shelldeck_browser::{KeyEvent, KeyEventKind};
use smallvec::SmallVec;

/// CEF receives device-independent viewport coordinates. Window scale belongs
/// in Resize, not in each pointer coordinate (which would double it on HiDPI).
pub(super) fn viewport_point(
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
    captured: bool,
) -> Option<(i32, i32)> {
    let x = f32::from(position.x - bounds.origin.x);
    let y = f32::from(position.y - bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if !x.is_finite() || !y.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    if !captured && (x < 0.0 || y < 0.0 || x >= width || y >= height) {
        return None;
    }
    Some((x.round() as i32, y.round() as i32))
}

pub(super) fn event_modifiers(modifiers: Modifiers) -> u32 {
    // CEF EventFlags are stable across the three desktop platforms.
    (if modifiers.shift { 1 << 1 } else { 0 })
        | (if modifiers.control { 1 << 2 } else { 0 })
        | (if modifiers.alt { 1 << 3 } else { 0 })
        | (if modifiers.platform { 1 << 7 } else { 0 })
}

/// Windows virtual-key codes are CEF's platform-independent keyboard contract.
/// Native codes are unavailable from GPUI; sending a guessed X11 keycode would
/// be incorrect on Wayland, macOS, and non-QWERTY layouts.
pub(super) fn virtual_key(key: &str) -> i32 {
    match key {
        "backspace" => 8,
        "tab" => 9,
        "enter" | "return" => 13,
        "shift" => 16,
        "control" | "ctrl" => 17,
        "alt" => 18,
        "escape" => 27,
        "space" | " " => 32,
        "pageup" | "page-up" => 33,
        "pagedown" | "page-down" => 34,
        "end" => 35,
        "home" => 36,
        "left" => 37,
        "up" => 38,
        "right" => 39,
        "down" => 40,
        "insert" => 45,
        "delete" => 46,
        "super" | "cmd" | "command" => 91,
        ";" | ":" => 186,
        "=" | "+" => 187,
        "," | "<" => 188,
        "-" | "_" => 189,
        "." | ">" => 190,
        "/" | "?" => 191,
        "`" | "~" => 192,
        "[" | "{" => 219,
        "\\" | "|" => 220,
        "]" | "}" => 221,
        "'" | "\"" => 222,
        _ => {
            if key.len() == 1 {
                let byte = key.as_bytes()[0].to_ascii_uppercase();
                if byte.is_ascii_alphanumeric() {
                    return byte as i32;
                }
            }
            key.strip_prefix('f')
                .and_then(|number| number.parse::<i32>().ok())
                .filter(|number| (1..=24).contains(number))
                .map(|number| number + 111)
                .unwrap_or(0)
        }
    }
}

/// Chromium requires a character event for Return's default action (implicit
/// form submission, contenteditable/textarea newline) on every platform. GPUI
/// consumes that control key, so it cannot arrive through the text handler.
/// Ordinary printable text continues through IME commit only, once.
pub(super) fn key_sequence(stroke: &Keystroke, released: bool) -> SmallVec<[KeyEvent; 2]> {
    let mut events = SmallVec::new();
    let modifiers = event_modifiers(stroke.modifiers);
    events.push(KeyEvent {
        kind: if released {
            KeyEventKind::KeyUp
        } else {
            KeyEventKind::RawKeyDown
        },
        windows_key_code: virtual_key(&stroke.key),
        native_key_code: 0,
        modifiers,
        character: None,
        unmodified_character: None,
        is_system_key: stroke.modifiers.alt,
    });
    if !released
        && matches!(stroke.key.as_str(), "enter" | "return")
        && !stroke.modifiers.control
        && !stroke.modifiers.alt
        && !stroke.modifiers.platform
    {
        events.push(KeyEvent {
            kind: KeyEventKind::Char,
            windows_key_code: 13,
            native_key_code: 0,
            modifiers,
            character: Some(13),
            unmodified_character: Some(13),
            is_system_key: false,
        });
    }
    events
}

pub(super) fn utf16_slice(text: &str, range: std::ops::Range<usize>) -> Option<String> {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let selected = units.get(range)?;
    String::from_utf16(selected).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    // SDTEST-2570 / SDUC-557: browser pointer coordinates exclude all surrounding
    // application chrome and remain logical pixels at any display scale.
    #[test]
    fn sdtest_2570_pointer_uses_actual_viewport_and_preserves_captured_drag() {
        let viewport = Bounds::new(point(px(350.0), px(160.0)), size(px(640.0), px(480.0)));
        assert_eq!(
            viewport_point(viewport, point(px(370.0), px(190.0)), false),
            Some((20, 30))
        );
        assert_eq!(
            viewport_point(viewport, point(px(340.0), px(190.0)), false),
            None
        );
        assert_eq!(
            viewport_point(viewport, point(px(1000.0), px(650.0)), true),
            Some((650, 490))
        );
        assert_eq!(
            viewport_point(viewport, point(px(990.0), px(190.0)), false),
            None
        );
    }

    // SDTEST-2571 / SDUC-557: IME range requests use UTF-16, including surrogate
    // pairs; malformed half-surrogate requests must not return corrupt text.
    #[test]
    fn sdtest_2571_ime_utf16_ranges_preserve_multilingual_text() {
        assert_eq!(utf16_slice("a😀é界", 1..3), Some("😀".into()));
        assert_eq!(utf16_slice("a😀é界", 3..5), Some("é界".into()));
        assert_eq!(utf16_slice("a😀é界", 1..2), None);
        assert_eq!(utf16_slice("a😀é界", 1..99), None);
    }

    // SDTEST-2572 / SDUC-557: navigation/editing keys retain browser semantics,
    // while unknown Unicode keys are inserted only through the input handler.
    #[test]
    fn sdtest_2572_virtual_keys_cover_browser_shortcuts_without_guessing_unicode() {
        assert_eq!(virtual_key("left"), 37);
        assert_eq!(virtual_key("delete"), 46);
        assert_eq!(virtual_key("f12"), 123);
        assert_eq!(virtual_key("f25"), 0);
        assert_eq!(virtual_key("q"), 81);
        assert_eq!(virtual_key("é"), 0);
        assert_eq!(
            event_modifiers(Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            }),
            6
        );
    }

    // SDTEST-2606 / SDUC-557: Return's default browser action requires exactly
    // RawDown -> CHAR(13) -> Up; printable text and modified shortcuts must not
    // acquire an extra CHAR beside the native input handler's text commit.
    #[test]
    fn sdtest_2606_human_enter_submits_without_duplicate_text_or_shortcut_chars() {
        for shift in [false, true] {
            let stroke = Keystroke {
                key: "enter".into(),
                key_char: Some("\r".into()),
                modifiers: Modifiers {
                    shift,
                    ..Default::default()
                },
            };
            let mut events = key_sequence(&stroke, false);
            events.extend(key_sequence(&stroke, true));
            assert!(matches!(
                events.as_slice(),
                [
                    KeyEvent {
                        kind: KeyEventKind::RawKeyDown,
                        ..
                    },
                    KeyEvent {
                        kind: KeyEventKind::Char,
                        character: Some(13),
                        ..
                    },
                    KeyEvent {
                        kind: KeyEventKind::KeyUp,
                        ..
                    }
                ]
            ));
            assert_eq!(
                events[1].modifiers,
                if shift {
                    shelldeck_browser::modifiers::SHIFT
                } else {
                    0
                }
            );
        }
        for modifiers in [
            Modifiers {
                control: true,
                ..Default::default()
            },
            Modifiers {
                alt: true,
                ..Default::default()
            },
            Modifiers {
                platform: true,
                ..Default::default()
            },
        ] {
            let events = key_sequence(
                &Keystroke {
                    key: "enter".into(),
                    key_char: Some("\r".into()),
                    modifiers,
                },
                false,
            );
            assert!(matches!(
                events.as_slice(),
                [KeyEvent {
                    kind: KeyEventKind::RawKeyDown,
                    character: None,
                    ..
                }]
            ));
        }
        for text in ["a", "é", "😀"] {
            let events = key_sequence(
                &Keystroke {
                    key: text.into(),
                    key_char: Some(text.into()),
                    modifiers: Modifiers::default(),
                },
                false,
            );
            assert!(matches!(
                events.as_slice(),
                [KeyEvent {
                    kind: KeyEventKind::RawKeyDown,
                    character: None,
                    ..
                }]
            ));
        }
    }
}
