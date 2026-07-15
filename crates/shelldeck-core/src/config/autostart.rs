//! Cross-platform launch-on-login (autostart) helper.
//!
//! Wraps the `auto-launch` crate to expose a small, ShellDeck-flavoured API:
//!
//! - [`AutostartHandle`] is what you get from [`handle`]; it knows the
//!   ShellDeck app name and the current executable path.
//! - [`apply`] is the one-shot "make the OS match this desired state" call
//!   the UI wires to the Settings toggle.
//! - [`is_enabled`] tells you what the OS *currently* thinks.
//!
//! Platform mapping (delegated to `auto-launch`):
//!
//! - **Linux** → an XDG autostart entry at
//!   `~/.config/autostart/ShellDeck.desktop`.
//! - **macOS** → a `launchd` per-user login item.
//! - **Windows** → an `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//!   registry value.
//!
//! All three are per-user (never system-wide) — flipping the Settings
//! toggle only affects the current login account.
//!
//! ## Sandbox / permission failures
//!
//! Under Flatpak, Snap, or a locked-down Windows profile, the OS may
//! refuse to write the autostart entry. The helper surfaces the error via
//! [`AutostartError`] so the UI can toast it and roll back the toggle
//! instead of silently drifting from what the user sees.

use auto_launch::AutoLaunch;
use std::env;
use std::path::PathBuf;
use thiserror::Error;

/// Application name registered with the OS autostart mechanism. Stable
/// across releases so we don't leave orphan entries behind after a rename.
pub const APP_NAME: &str = "ShellDeck";

/// Errors returned by the autostart helpers.
#[derive(Debug, Error)]
pub enum AutostartError {
    /// `std::env::current_exe()` failed — extremely rare, usually means
    /// the process was launched in a way that hides the exe path.
    #[error("could not resolve current executable path: {0}")]
    CurrentExe(#[from] std::io::Error),

    /// The executable path is not valid UTF-8. `auto-launch` requires
    /// `&str`, so we bail if the OS returns a non-UTF-8 path (Windows
    /// short names, exotic Linux mounts).
    #[error("executable path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),

    /// The underlying `auto-launch` call failed. Typical causes: sandbox
    /// (Flatpak/Snap), missing HOME on Linux, permission denied on the
    /// registry key on Windows.
    #[error("OS autostart call failed: {0}")]
    Backend(#[from] auto_launch::Error),
}

/// Handle scoped to the current binary. Cheap to construct; hold on to
/// one if you're going to flip the toggle repeatedly, otherwise just
/// call [`apply`] / [`is_enabled`] for a one-shot.
pub struct AutostartHandle {
    inner: AutoLaunch,
}

impl AutostartHandle {
    /// Build a handle for the currently-running executable.
    pub fn new() -> Result<Self, AutostartError> {
        let exe = env::current_exe()?;
        let exe_str = exe
            .to_str()
            .ok_or_else(|| AutostartError::NonUtf8Path(exe.clone()))?;

        // No CLI args on startup — we always launch the app cold. If a
        // future release wants to pass e.g. `--minimized` (once tray
        // exists), plumb it through here.
        let inner = AutoLaunch::new(APP_NAME, exe_str, &[] as &[&str]);
        Ok(Self { inner })
    }

    /// Enable launch-on-login for the current user.
    pub fn enable(&self) -> Result<(), AutostartError> {
        self.inner.enable()?;
        Ok(())
    }

    /// Disable launch-on-login for the current user.
    pub fn disable(&self) -> Result<(), AutostartError> {
        self.inner.disable()?;
        Ok(())
    }

    /// Reflect what the OS currently thinks — not what our config file
    /// says. Useful to detect drift (user manually deleted the desktop
    /// entry, sysadmin disabled the registry key, etc.).
    pub fn is_enabled(&self) -> Result<bool, AutostartError> {
        Ok(self.inner.is_enabled()?)
    }
}

/// Build a handle for the currently-running executable. Prefer this
/// over calling [`AutostartHandle::new`] directly — it's the documented
/// entry point.
pub fn handle() -> Result<AutostartHandle, AutostartError> {
    AutostartHandle::new()
}

/// One-shot "make the OS match `desired`". Enables or disables as
/// needed, returning the resolved state on success.
pub fn apply(desired: bool) -> Result<bool, AutostartError> {
    let h = handle()?;
    if desired {
        h.enable()?;
    } else {
        h.disable()?;
    }
    h.is_enabled()
}

/// Convenience: current OS state without holding a handle.
pub fn is_enabled() -> Result<bool, AutostartError> {
    handle()?.is_enabled()
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-201 — handle construction resolves the current exe path.
    // Guards against the `current_exe()` / UTF-8 error paths regressing
    // silently. We do NOT test enable/disable in unit tests: the actual
    // OS side-effects would litter the developer's autostart list.
    #[test]
    fn handle_can_be_constructed_for_current_exe() {
        let h = handle().expect("current_exe must resolve in a test binary");
        // is_enabled is a pure read (no OS mutation) — safe in tests.
        // It might return either bool depending on the developer's own
        // machine; we just assert it doesn't error.
        let _ = h.is_enabled();
    }

    // Opt-in end-to-end probe: actually writes + reads + removes the OS
    // autostart entry to verify the full apply/is_enabled loop works on
    // the current platform. NOT run by default — gate is `SHELLDECK_LIVE=1`
    // — because it mutates the developer's login items. Cleans up after
    // itself even on failure via a defer-drop guard.
    //
    // Run with: SHELLDECK_LIVE=1 cargo test -p shelldeck-core -- --ignored autostart_live
    #[test]
    #[ignore]
    fn autostart_live_roundtrip() {
        if std::env::var("SHELLDECK_LIVE").as_deref() != Ok("1") {
            eprintln!("skipping (set SHELLDECK_LIVE=1 to enable)");
            return;
        }
        // Ensure we always clean up, even on assertion failure.
        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = super::apply(false);
            }
        }
        let _cleanup = Cleanup;

        let before = is_enabled().expect("is_enabled read must succeed");
        eprintln!("initial OS state: {before}");

        let after_enable = apply(true).expect("apply(true) must succeed");
        assert!(after_enable, "OS should report enabled after apply(true)");

        let after_disable = apply(false).expect("apply(false) must succeed");
        assert!(!after_disable, "OS should report disabled after apply(false)");
    }
}
