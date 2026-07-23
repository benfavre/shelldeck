#![allow(dead_code)]

use anyhow::Result;
use collections::HashMap;

use crate::Keystroke;

pub struct LinuxGlobalHotkey {
    registered: HashMap<u32, Keystroke>,
}

impl LinuxGlobalHotkey {
    pub fn new() -> Self {
        Self {
            registered: HashMap::default(),
        }
    }

    pub fn register(&mut self, id: u32, keystroke: &Keystroke) -> Result<()> {
        self.registered.insert(id, keystroke.clone());
        Ok(())
    }

    pub fn unregister(&mut self, id: u32) {
        self.registered.remove(&id);
    }
}

#[cfg(feature = "x11")]
pub mod x11 {
    use super::*;
    use std::rc::Rc;
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{self, ConnectionExt as _, GrabMode, ModMask};
    use x11rb::xcb_ffi::XCBConnection;

    fn keystroke_to_x11_modmask(keystroke: &Keystroke) -> u16 {
        let mut mask = 0u16;
        if keystroke.modifiers.control {
            mask |= u16::from(ModMask::CONTROL);
        }
        if keystroke.modifiers.alt {
            mask |= u16::from(ModMask::M1);
        }
        if keystroke.modifiers.shift {
            mask |= u16::from(ModMask::SHIFT);
        }
        if keystroke.modifiers.platform {
            mask |= u16::from(ModMask::M4);
        }
        mask
    }

    // ShellDeck patch: global grabs must survive Caps Lock and Num Lock state.
    fn lock_variants(modmask: u16) -> [u16; 4] {
        let caps_lock = u16::from(ModMask::LOCK);
        let num_lock = u16::from(ModMask::M2);
        [
            modmask,
            modmask | caps_lock,
            modmask | num_lock,
            modmask | caps_lock | num_lock,
        ]
    }

    fn modifiers_match(actual: u16, registered: u16) -> bool {
        let ignored_locks = u16::from(ModMask::LOCK) | u16::from(ModMask::M2);
        actual & !ignored_locks == registered
    }

    fn keystroke_to_x11_keycode(keystroke: &Keystroke, xcb: &XCBConnection) -> Option<u8> {
        let setup = xcb.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let keysym = key_name_to_keysym(&keystroke.key)?;

        let reply = xcb
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)
            .ok()?
            .reply()
            .ok()?;

        let keysyms_per_keycode = reply.keysyms_per_keycode as usize;
        if keysyms_per_keycode == 0 {
            return None;
        }

        for i in 0..((max_keycode - min_keycode + 1) as usize) {
            let base = i * keysyms_per_keycode;
            for j in 0..keysyms_per_keycode {
                if reply.keysyms[base + j] == keysym {
                    return Some(min_keycode + i as u8);
                }
            }
        }

        None
    }

    fn key_name_to_keysym(key: &str) -> Option<u32> {
        match key.to_lowercase().as_str() {
            "a" => Some(0x61),
            "b" => Some(0x62),
            "c" => Some(0x63),
            "d" => Some(0x64),
            "e" => Some(0x65),
            "f" => Some(0x66),
            "g" => Some(0x67),
            "h" => Some(0x68),
            "i" => Some(0x69),
            "j" => Some(0x6a),
            "k" => Some(0x6b),
            "l" => Some(0x6c),
            "m" => Some(0x6d),
            "n" => Some(0x6e),
            "o" => Some(0x6f),
            "p" => Some(0x70),
            "q" => Some(0x71),
            "r" => Some(0x72),
            "s" => Some(0x73),
            "t" => Some(0x74),
            "u" => Some(0x75),
            "v" => Some(0x76),
            "w" => Some(0x77),
            "x" => Some(0x78),
            "y" => Some(0x79),
            "z" => Some(0x7a),
            "0" => Some(0x30),
            "1" => Some(0x31),
            "2" => Some(0x32),
            "3" => Some(0x33),
            "4" => Some(0x34),
            "5" => Some(0x35),
            "6" => Some(0x36),
            "7" => Some(0x37),
            "8" => Some(0x38),
            "9" => Some(0x39),
            "space" => Some(0x20),
            "enter" | "return" => Some(0xff0d),
            "tab" => Some(0xff09),
            "escape" => Some(0xff1b),
            "backspace" => Some(0xff08),
            "delete" => Some(0xffff),
            "insert" => Some(0xff63),
            "home" => Some(0xff50),
            "end" => Some(0xff57),
            "pageup" => Some(0xff55),
            "pagedown" => Some(0xff56),
            "left" => Some(0xff51),
            "up" => Some(0xff52),
            "right" => Some(0xff53),
            "down" => Some(0xff54),
            "f1" => Some(0xffbe),
            "f2" => Some(0xffbf),
            "f3" => Some(0xffc0),
            "f4" => Some(0xffc1),
            "f5" => Some(0xffc2),
            "f6" => Some(0xffc3),
            "f7" => Some(0xffc4),
            "f8" => Some(0xffc5),
            "f9" => Some(0xffc6),
            "f10" => Some(0xffc7),
            "f11" => Some(0xffc8),
            "f12" => Some(0xffc9),
            "-" => Some(0x2d),
            "=" => Some(0x3d),
            "[" => Some(0x5b),
            "]" => Some(0x5d),
            "\\" => Some(0x5c),
            ";" => Some(0x3b),
            "'" => Some(0x27),
            "`" => Some(0x60),
            "," => Some(0x2c),
            "." => Some(0x2e),
            "/" => Some(0x2f),
            _ => None,
        }
    }

    pub struct X11GlobalHotkey {
        inner: LinuxGlobalHotkey,
        keycodes: HashMap<u32, (u8, u16)>,
    }

    impl X11GlobalHotkey {
        pub fn new() -> Self {
            Self {
                inner: LinuxGlobalHotkey::new(),
                keycodes: HashMap::default(),
            }
        }

        pub fn register(
            &mut self,
            id: u32,
            keystroke: &Keystroke,
            xcb: &Rc<XCBConnection>,
            root_window: xproto::Window,
        ) -> Result<()> {
            let keycode = keystroke_to_x11_keycode(keystroke, xcb).ok_or_else(|| {
                anyhow::anyhow!("Could not resolve keycode for key: {}", keystroke.key)
            })?;
            let modmask = keystroke_to_x11_modmask(keystroke);

            // ShellDeck patch: grab every lock-state variant and roll back partial grabs.
            let mut grabbed: Vec<u16> = Vec::new();
            for variant in lock_variants(modmask) {
                let result = xcb
                    .grab_key(
                        false,
                        root_window,
                        variant.into(),
                        keycode,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )?
                    .check();
                if let Err(error) = result {
                    for grabbed_variant in grabbed {
                        let _ = xcb.ungrab_key(keycode, root_window, grabbed_variant.into());
                    }
                    return Err(error.into());
                }
                grabbed.push(variant);
            }

            self.keycodes.insert(id, (keycode, modmask));
            self.inner.register(id, keystroke)
        }

        // ShellDeck patch: map root-window KeyPress events back to registered IDs.
        pub fn matching_id(&self, keycode: u8, modifiers: u16) -> Option<u32> {
            self.keycodes.iter().find_map(|(id, (registered_keycode, registered_modifiers))| {
                (*registered_keycode == keycode
                    && modifiers_match(modifiers, *registered_modifiers))
                .then_some(*id)
            })
        }

        pub fn unregister(
            &mut self,
            id: u32,
            xcb: &Rc<XCBConnection>,
            root_window: xproto::Window,
        ) {
            if let Some((keycode, modmask)) = self.keycodes.remove(&id) {
                // ShellDeck patch: release every lock-state grab registered above.
                for variant in lock_variants(modmask) {
                    let _ = xcb.ungrab_key(keycode, root_window, variant.into());
                }
            }
            self.inner.unregister(id);
        }
    }

    // ShellDeck patch: protect lock-state matching against regressions.
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn matching_modifiers_ignore_caps_and_num_lock() {
            let shortcut = u16::from(ModMask::CONTROL) | u16::from(ModMask::SHIFT);
            assert!(modifiers_match(shortcut, shortcut));
            assert!(modifiers_match(
                shortcut | u16::from(ModMask::LOCK) | u16::from(ModMask::M2),
                shortcut
            ));
            assert!(!modifiers_match(
                shortcut | u16::from(ModMask::M1),
                shortcut
            ));
        }
    }
}

#[cfg(feature = "wayland")]
pub mod wayland {
    use super::*;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use calloop::channel::Sender;
    use futures::StreamExt as _;
    use std::time::Duration;

    use crate::{BackgroundExecutor, Task};

    // ShellDeck patch: bridge GPUI global-hotkey registrations through the
    // XDG Global Shortcuts portal when Wayland forbids compositor key grabs.
    const PORTAL_REGISTRATION_DEBOUNCE: Duration = Duration::from_millis(100);
    const PORTAL_SHORTCUT_PREFIX: &str = "gpui-";

    fn portal_shortcut_id(id: u32) -> String {
        format!("{PORTAL_SHORTCUT_PREFIX}{id}")
    }

    fn id_from_portal_shortcut(shortcut_id: &str) -> Option<u32> {
        shortcut_id
            .strip_prefix(PORTAL_SHORTCUT_PREFIX)?
            .parse()
            .ok()
    }

    fn portal_key_name(key: &str) -> Option<String> {
        if key.len() == 1 && key.chars().all(|character| character.is_ascii_alphanumeric()) {
            return Some(key.to_ascii_lowercase());
        }
        let name = match key.to_ascii_lowercase().as_str() {
            "space" => "space",
            "enter" | "return" => "Return",
            "tab" => "Tab",
            "escape" => "Escape",
            "backspace" => "BackSpace",
            "delete" => "Delete",
            "insert" => "Insert",
            "home" => "Home",
            "end" => "End",
            "pageup" => "Page_Up",
            "pagedown" => "Page_Down",
            "left" => "Left",
            "up" => "Up",
            "right" => "Right",
            "down" => "Down",
            "-" => "minus",
            "=" => "equal",
            "[" => "bracketleft",
            "]" => "bracketright",
            "\\" => "backslash",
            ";" => "semicolon",
            "'" => "apostrophe",
            "`" => "grave",
            "," => "comma",
            "." => "period",
            "/" => "slash",
            key if key.len() > 1
                && key.starts_with('f')
                && key[1..]
                    .parse::<u8>()
                    .is_ok_and(|number| (1..=35).contains(&number)) =>
            {
                return Some(key.to_ascii_uppercase());
            }
            _ => return None,
        };
        Some(name.to_string())
    }

    fn keystroke_to_portal_trigger(keystroke: &Keystroke) -> Result<String> {
        let mut parts = Vec::with_capacity(5);
        if keystroke.modifiers.control {
            parts.push("CTRL".to_string());
        }
        if keystroke.modifiers.alt {
            parts.push("ALT".to_string());
        }
        if keystroke.modifiers.shift {
            parts.push("SHIFT".to_string());
        }
        if keystroke.modifiers.platform {
            parts.push("LOGO".to_string());
        }
        parts.push(portal_key_name(&keystroke.key).ok_or_else(|| {
            anyhow::anyhow!(
                "Unsupported key for Wayland global shortcut: {}",
                keystroke.key
            )
        })?);
        Ok(parts.join("+"))
    }

    async fn run_portal_session(
        registrations: Vec<(u32, Keystroke)>,
        activation_tx: Sender<u32>,
    ) -> Result<()> {
        let proxy = GlobalShortcuts::new().await?;
        let session = proxy.create_session().await?;
        let shortcuts = registrations
            .into_iter()
            .map(|(id, keystroke)| {
                let trigger = keystroke_to_portal_trigger(&keystroke)?;
                let description = format!("Global shortcut {trigger}");
                Ok(NewShortcut::new(portal_shortcut_id(id), description)
                    .preferred_trigger(Some(trigger.as_str())))
            })
            .collect::<Result<Vec<_>>>()?;
        let request = proxy.bind_shortcuts(&session, &shortcuts, None).await?;
        let bound = request.response()?;
        if bound.shortcuts().is_empty() {
            return Err(anyhow::anyhow!(
                "Wayland Global Shortcuts portal did not bind any shortcut"
            ));
        }

        log::info!(
            "Wayland Global Shortcuts portal bound {} shortcut(s)",
            bound.shortcuts().len()
        );
        let mut activated = proxy.receive_activated().await?;
        while let Some(event) = activated.next().await {
            let Some(id) = id_from_portal_shortcut(event.shortcut_id()) else {
                continue;
            };
            if activation_tx.send(id).is_err() {
                break;
            }
        }
        Ok(())
    }

    pub struct WaylandGlobalHotkey {
        inner: LinuxGlobalHotkey,
        background_executor: BackgroundExecutor,
        activation_tx: Sender<u32>,
        portal_task: Option<Task<()>>,
    }

    impl WaylandGlobalHotkey {
        pub fn new(background_executor: BackgroundExecutor, activation_tx: Sender<u32>) -> Self {
            Self {
                inner: LinuxGlobalHotkey::new(),
                background_executor,
                activation_tx,
                portal_task: None,
            }
        }

        pub fn register(&mut self, id: u32, keystroke: &Keystroke) -> Result<()> {
            keystroke_to_portal_trigger(keystroke)?;
            self.inner.register(id, keystroke)?;
            self.schedule_portal_session();
            Ok(())
        }

        pub fn unregister(&mut self, id: u32) {
            self.inner.unregister(id);
            self.schedule_portal_session();
        }

        fn schedule_portal_session(&mut self) {
            let mut registrations = self
                .inner
                .registered
                .iter()
                .map(|(id, keystroke)| (*id, keystroke.clone()))
                .collect::<Vec<_>>();
            registrations.sort_by_key(|(id, _)| *id);
            if registrations.is_empty() {
                self.portal_task = None;
                return;
            }

            let executor = self.background_executor.clone();
            let activation_tx = self.activation_tx.clone();
            self.portal_task = Some(self.background_executor.spawn(async move {
                executor.timer(PORTAL_REGISTRATION_DEBOUNCE).await;
                if let Err(error) = run_portal_session(registrations, activation_tx).await {
                    log::warn!("Wayland Global Shortcuts portal unavailable: {error:#}");
                }
            }));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::Keystroke;

        #[test]
        fn portal_trigger_uses_xdg_shortcut_syntax() {
            let shortcut = Keystroke::parse("ctrl-shift-space").expect("valid shortcut");
            assert_eq!(
                keystroke_to_portal_trigger(&shortcut).unwrap(),
                "CTRL+SHIFT+space"
            );

            let shortcut = Keystroke::parse("alt-super-f12").expect("valid shortcut");
            assert_eq!(
                keystroke_to_portal_trigger(&shortcut).unwrap(),
                "ALT+LOGO+F12"
            );
        }

        #[test]
        fn portal_ids_round_trip_and_reject_foreign_values() {
            assert_eq!(id_from_portal_shortcut(&portal_shortcut_id(42)), Some(42));
            assert_eq!(id_from_portal_shortcut("other-42"), None);
            assert_eq!(id_from_portal_shortcut("gpui-not-a-number"), None);
        }

        #[test]
        fn unsupported_portal_key_is_rejected_before_requesting_permission() {
            let shortcut = Keystroke {
                key: "💥".to_string(),
                ..Default::default()
            };
            assert!(keystroke_to_portal_trigger(&shortcut).is_err());
        }
    }
}
