use std::collections::HashMap;

use anyhow::Result;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::*,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD,
                NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
            },
            WindowsAndMessaging::*,
        },
    },
};

use crate::{SharedString, TrayMenuItem, WM_GPUI_TRAY_ICON};

const TRAY_ICON_ID: u32 = 1;

pub(crate) struct WindowsTray {
    icon_added: bool,
    hwnd: HWND,
    current_icon: Option<HICON>,
    pub(crate) menu_items: Vec<TrayMenuItem>,
    pub(crate) command_id_map: HashMap<u32, SharedString>,
}

impl WindowsTray {
    // ShellDeck patch: SDPATCH-120 — create the shell entry only after a real
    // HICON exists, include it in the authoritative NIM_ADD, and retain the
    // exact add result as the hidden-start availability boundary.
    pub fn new(hwnd: HWND) -> Self {
        Self {
            icon_added: false,
            hwnd,
            current_icon: None,
            menu_items: Vec::new(),
            command_id_map: HashMap::new(),
        }
    }

    pub(crate) fn is_available(&self) -> bool {
        self.icon_added && self.current_icon.is_some()
    }

    pub fn set_icon(&mut self, icon_data: Option<&[u8]>, hwnd: HWND) {
        let Some(icon_data) = icon_data else {
            self.remove_icon();
            return;
        };
        let Some(hicon) = create_hicon_from_bytes(icon_data) else {
            return;
        };

        if self.icon_added {
            let modify = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                uFlags: NIF_ICON,
                hIcon: hicon,
                ..Default::default()
            };
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &modify).as_bool() } {
                self.replace_retained_icon(hicon);
                return;
            }

            // Explorer may have restarted and forgotten the old entry. Remove
            // any surviving registration before retrying one authoritative add.
            self.remove_icon();
        }

        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_SHOWTIP | NIF_ICON,
            uCallbackMessage: WM_GPUI_TRAY_ICON,
            hIcon: hicon,
            ..Default::default()
        };
        if unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() } {
            self.icon_added = true;
            self.replace_retained_icon(hicon);
        } else {
            unsafe {
                let _ = DestroyIcon(hicon);
            }
        }
    }

    fn replace_retained_icon(&mut self, hicon: HICON) {
        if let Some(old_icon) = self.current_icon.replace(hicon) {
            unsafe {
                let _ = DestroyIcon(old_icon);
            }
        }
    }

    // ShellDeck patch: SDPATCH-120 — Explorer discards notification-area
    // registrations when it restarts. Re-add the retained HICON on the
    // TaskbarCreated broadcast and make that exact NIM_ADD result authoritative.
    pub(crate) fn handle_taskbar_created(&mut self) -> bool {
        self.handle_taskbar_created_with(|nid| unsafe {
            Shell_NotifyIconW(NIM_ADD, nid).as_bool()
        })
    }

    fn handle_taskbar_created_with(
        &mut self,
        add_icon: impl FnOnce(&NOTIFYICONDATAW) -> bool,
    ) -> bool {
        self.icon_added = false;
        let Some(hicon) = self.current_icon else {
            return false;
        };
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_SHOWTIP | NIF_ICON,
            uCallbackMessage: WM_GPUI_TRAY_ICON,
            hIcon: hicon,
            ..Default::default()
        };
        self.icon_added = add_icon(&nid);
        self.icon_added
    }

    fn remove_icon(&mut self) {
        if self.icon_added {
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: TRAY_ICON_ID,
                ..Default::default()
            };
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }
            self.icon_added = false;
        }
        if let Some(icon) = self.current_icon.take() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }

    pub fn set_tooltip(&mut self, tooltip: &str, hwnd: HWND) {
        if !self.is_available() {
            return;
        }
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_TIP,
            ..Default::default()
        };
        let wide: Vec<u16> = tooltip.encode_utf16().collect();
        let len = wide.len().min(nid.szTip.len() - 1);
        nid.szTip[..len].copy_from_slice(&wide[..len]);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn show_balloon(&self, title: &str, body: &str, hwnd: HWND) -> Result<()> {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_INFO,
            ..Default::default()
        };

        let title_wide: Vec<u16> = title.encode_utf16().collect();
        let title_len = title_wide.len().min(nid.szInfoTitle.len() - 1);
        nid.szInfoTitle[..title_len].copy_from_slice(&title_wide[..title_len]);

        let body_wide: Vec<u16> = body.encode_utf16().collect();
        let body_len = body_wide.len().min(nid.szInfo.len() - 1);
        nid.szInfo[..body_len].copy_from_slice(&body_wide[..body_len]);

        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid)
                .ok()
                .map_err(|e| anyhow::anyhow!("Failed to show balloon notification: {}", e))
        }
    }

    pub fn show_context_menu(&mut self, hwnd: HWND) {
        if self.menu_items.is_empty() {
            return;
        }
        self.command_id_map.clear();
        unsafe {
            let hmenu = CreatePopupMenu();
            if let Ok(hmenu) = hmenu {
                let mut counter: u32 = 1;
                Self::build_menu(
                    hmenu,
                    &self.menu_items,
                    &mut counter,
                    &mut self.command_id_map,
                );
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let _ = SetForegroundWindow(hwnd);
                let _ = TrackPopupMenu(
                    hmenu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN,
                    point.x,
                    point.y,
                    None,
                    hwnd,
                    None,
                );
                let _ = DestroyMenu(hmenu);
            }
        }
    }

    pub(crate) unsafe fn build_menu(
        hmenu: HMENU,
        items: &[TrayMenuItem],
        counter: &mut u32,
        id_map: &mut HashMap<u32, SharedString>,
    ) {
        for item in items.iter() {
            match item {
                TrayMenuItem::Action { label, id } => {
                    let cmd_id = *counter;
                    *counter += 1;
                    id_map.insert(cmd_id, id.clone());
                    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    unsafe {
                        let _ =
                            AppendMenuW(hmenu, MF_STRING, cmd_id as usize, PCWSTR(wide.as_ptr()));
                    }
                }
                // ShellDeck patch: SDPATCH-120 — render informational rows disabled.
                TrayMenuItem::Label { label } => {
                    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    unsafe {
                        let _ = AppendMenuW(
                            hmenu,
                            MF_STRING | MF_DISABLED,
                            0,
                            PCWSTR(wide.as_ptr()),
                        );
                    }
                }
                TrayMenuItem::Separator => unsafe {
                    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
                },
                TrayMenuItem::Submenu {
                    label,
                    items: sub_items,
                } => {
                    if let Ok(submenu) = unsafe { CreatePopupMenu() } {
                        unsafe { Self::build_menu(submenu, sub_items, counter, id_map) };
                        let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                        unsafe {
                            let _ = AppendMenuW(
                                hmenu,
                                MF_POPUP,
                                submenu.0 as usize,
                                PCWSTR(wide.as_ptr()),
                            );
                        }
                    }
                }
                TrayMenuItem::Toggle {
                    label, checked, id, ..
                } => {
                    let cmd_id = *counter;
                    *counter += 1;
                    id_map.insert(cmd_id, id.clone());
                    let flags = if *checked {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    unsafe {
                        let _ = AppendMenuW(hmenu, flags, cmd_id as usize, PCWSTR(wide.as_ptr()));
                    }
                }
            }
        }
    }
}

impl Drop for WindowsTray {
    fn drop(&mut self) {
        self.remove_icon();
    }
}

// ShellDeck patch: SDPATCH-120 — decode a complete .ico file as an ICONDIR,
// select its closest high-depth image, and pass only that image resource to
// CreateIconFromResourceEx. LookupIconIdFromDirectoryEx returns a resource ID,
// not an offset into an .ico file, and cannot decode PNG bytes directly.
fn create_hicon_from_bytes(data: &[u8]) -> Option<HICON> {
    let resource = ico_resource(data, 32)?;
    unsafe {
        CreateIconFromResourceEx(resource, true, 0x00030000, 32, 32, LR_DEFAULTCOLOR).ok()
    }
}

fn ico_resource(data: &[u8], desired_size: u16) -> Option<&[u8]> {
    if read_u16(data, 0)? != 0 || read_u16(data, 2)? != 1 {
        return None;
    }
    let count = usize::from(read_u16(data, 4)?);
    let directory_end = 6usize.checked_add(count.checked_mul(16)?)?;
    if count == 0 || directory_end > data.len() {
        return None;
    }

    let mut best: Option<(u32, u16, &[u8])> = None;
    for index in 0..count {
        let entry = 6usize.checked_add(index.checked_mul(16)?)?;
        let width = match *data.get(entry)? {
            0 => 256,
            width => u16::from(width),
        };
        let height = match *data.get(entry + 1)? {
            0 => 256,
            height => u16::from(height),
        };
        let bit_depth = read_u16(data, entry + 6)?;
        let length = usize::try_from(read_u32(data, entry + 8)?).ok()?;
        let offset = usize::try_from(read_u32(data, entry + 12)?).ok()?;
        let end = offset.checked_add(length)?;
        if length == 0 || offset < directory_end || end > data.len() {
            return None;
        }
        let distance = u32::from(width.abs_diff(desired_size))
            .checked_add(u32::from(height.abs_diff(desired_size)))?;
        let resource = data.get(offset..end)?;
        if best
            .as_ref()
            .is_none_or(|(best_distance, best_depth, _)| {
                distance < *best_distance
                    || (distance == *best_distance && bit_depth > *best_depth)
            })
        {
            best = Some((distance, bit_depth, resource));
        }
    }
    best.map(|(_, _, resource)| resource)
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(offset..offset + 2)?.try_into().ok()?))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?))
}

// ShellDeck patch: SDPATCH-120 — pin Explorer-restart custody, ICO selection,
// and the real ICO-resource-to-HICON boundary on native Windows CI.
#[cfg(test)]
mod tests {
    use std::mem::ManuallyDrop;

    use super::*;

    // SDTEST-1813
    #[test]
    fn sdtest_1813_explorer_restart_readd_is_authoritative_and_retains_icon() {
        let hwnd = HWND(1usize as *mut core::ffi::c_void);
        let hicon = HICON(2usize as *mut core::ffi::c_void);
        let mut tray = ManuallyDrop::new(WindowsTray {
            icon_added: true,
            hwnd,
            current_icon: Some(hicon),
            menu_items: Vec::new(),
            command_id_map: HashMap::new(),
        });

        assert!(!tray.handle_taskbar_created_with(|nid| {
            assert_eq!(nid.hWnd, hwnd);
            assert_eq!(nid.uID, TRAY_ICON_ID);
            assert_eq!(nid.hIcon, hicon);
            assert_eq!(nid.uCallbackMessage, WM_GPUI_TRAY_ICON);
            assert_eq!(nid.uFlags, NIF_MESSAGE | NIF_SHOWTIP | NIF_ICON);
            false
        }));
        assert!(!tray.is_available());
        assert_eq!(tray.current_icon, Some(hicon));

        assert!(tray.handle_taskbar_created_with(|_| true));
        assert!(tray.is_available());
        assert_eq!(tray.current_icon, Some(hicon));

        tray.current_icon = None;
        assert!(!tray.handle_taskbar_created_with(|_| panic!("no add without a retained HICON")));
        assert!(!tray.is_available());
    }

    fn test_ico(entries: &[(u8, u8, u16, &[u8])]) -> Vec<u8> {
        let directory_end = 6 + entries.len() * 16;
        let mut bytes = Vec::with_capacity(
            directory_end + entries.iter().map(|(_, _, _, data)| data.len()).sum::<usize>(),
        );
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let mut offset = directory_end;
        for &(width, height, depth, data) in entries {
            bytes.push(width);
            bytes.push(height);
            bytes.push(0);
            bytes.push(0);
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&depth.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += data.len();
        }
        for &(_, _, _, data) in entries {
            bytes.extend_from_slice(data);
        }
        bytes
    }

    // SDTEST-1814
    #[test]
    fn sdtest_1814_ico_resource_selects_closest_then_highest_depth() {
        let ico = test_ico(&[
            (16, 16, 32, &[0x16]),
            (32, 32, 8, &[0x08]),
            (32, 32, 32, &[0x32]),
            (48, 48, 32, &[0x48]),
        ]);
        assert_eq!(ico_resource(&ico, 32), Some(&[0x32][..]));

        let mut overlapping = test_ico(&[(32, 32, 32, &[0x32])]);
        overlapping[18..22].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(ico_resource(&overlapping, 32), None);
        assert_eq!(ico_resource(&[0, 0, 2, 0, 0, 0], 32), None);
    }

    // SDTEST-1815
    #[test]
    fn sdtest_1815_shell_deck_ico_creates_a_native_hicon() {
        let bytes = include_bytes!("../../../../../packaging/icons/shelldeck.ico");
        let hicon = create_hicon_from_bytes(bytes).expect("CreateIconFromResourceEx accepts ICO");
        assert_ne!(hicon, HICON::default());
        unsafe {
            DestroyIcon(hicon).expect("destroy test HICON");
        }
    }
}
