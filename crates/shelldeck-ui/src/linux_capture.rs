//! Interactive region capture on Linux without asking users to install a tool.
//!
//! 1. The desktop's Screenshot portal (`xdg-desktop-portal`): GNOME with its
//!    portal backend, KDE and wlroots desktops show their own selector.
//! 2. GNOME Shell's screenshot API, for GNOME sessions without the portal
//!    backend (Ubuntu 22.04 without `xdg-desktop-portal-gnome`). GNOME 41+
//!    only serves callers that own an allow-listed bus name; we claim
//!    `org.gnome.Screenshot` for the duration of the call, exactly as
//!    gnome-screenshot does, then release it.
//! 3. An installed capture tool (gnome-screenshot, spectacle, grim + slurp,
//!    ImageMagick `import`).
//! 4. The desktop's own screenshot key. Some installs (Ubuntu 22.04 without
//!    `xdg-desktop-portal-gnome`, minimal sessions) have neither of the above,
//!    but GNOME's Print Screen still saves a file under Pictures. A hint asks
//!    the user to press it and the new file is picked up.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

const WATCH_TIMEOUT: Duration = Duration::from_secs(90);
const WATCH_POLL: Duration = Duration::from_millis(400);

pub(crate) enum LinuxCapture {
    Captured(PathBuf),
    Cancelled,
}

static HINT_SINK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Installed by the binary: tells the user to take the screenshot with the
/// desktop's own key (an OS notification, since the window may be hidden).
pub fn set_capture_hint_sink(sink: Box<dyn Fn() + Send + Sync>) {
    let _ = HINT_SINK.set(sink);
}

pub(crate) fn capture(target: &Path) -> LinuxCapture {
    match portal_capture() {
        PortalOutcome::Saved(path) => return LinuxCapture::Captured(path),
        PortalOutcome::Cancelled => return LinuxCapture::Cancelled,
        PortalOutcome::Unavailable => {}
    }
    match gnome_shell_capture(target) {
        PortalOutcome::Saved(path) => return LinuxCapture::Captured(path),
        PortalOutcome::Cancelled => return LinuxCapture::Cancelled,
        PortalOutcome::Unavailable => {}
    }
    if let Some(captured) = tool_capture(target) {
        return if captured {
            LinuxCapture::Captured(target.to_path_buf())
        } else {
            LinuxCapture::Cancelled
        };
    }
    system_key_capture()
}

enum PortalOutcome {
    Saved(PathBuf),
    Cancelled,
    Unavailable,
}

fn portal_capture() -> PortalOutcome {
    use ashpd::desktop::screenshot::Screenshot;
    use ashpd::desktop::ResponseError;

    let result = futures_lite::future::block_on(async {
        Screenshot::request()
            .interactive(true)
            .modal(true)
            .send()
            .await?
            .response()
    });
    match result {
        Ok(response) => match response.uri().to_file_path() {
            Ok(path) if file_has_content(&path) => PortalOutcome::Saved(path),
            _ => {
                tracing::warn!("Screenshot portal returned an unreadable file");
                PortalOutcome::Unavailable
            }
        },
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => PortalOutcome::Cancelled,
        Err(error) => {
            tracing::info!("Screenshot portal unavailable: {error}");
            PortalOutcome::Unavailable
        }
    }
}

fn gnome_shell_capture(target: &Path) -> PortalOutcome {
    const SHELL: &str = "org.gnome.Shell.Screenshot";
    const PATH: &str = "/org/gnome/Shell/Screenshot";
    const ALLOWED_NAME: &str = "org.gnome.Screenshot";

    let Some(target_str) = target.to_str().map(str::to_string) else {
        return PortalOutcome::Unavailable;
    };
    futures_lite::future::block_on(async move {
        let conn = match zbus::Connection::session().await {
            Ok(conn) => conn,
            Err(error) => {
                tracing::info!("GNOME Shell screenshot: no session bus: {error}");
                return PortalOutcome::Unavailable;
            }
        };
        let shell_present = match (zbus::fdo::DBusProxy::new(&conn).await, SHELL.try_into()) {
            (Ok(dbus), Ok(name)) => dbus.name_has_owner(name).await.unwrap_or(false),
            _ => false,
        };
        if !shell_present {
            return PortalOutcome::Unavailable;
        }
        // Claim the allow-listed name only if free; a running gnome-screenshot keeps it.
        let claimed = conn
            .request_name_with_flags(ALLOWED_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .await
            .is_ok();
        let outcome = async {
            let area = conn
                .call_method(Some(SHELL), PATH, Some(SHELL), "SelectArea", &())
                .await;
            let (x, y, w, h): (i32, i32, i32, i32) = match area {
                Ok(reply) => match reply.body().deserialize() {
                    Ok(area) => area,
                    Err(_) => return PortalOutcome::Unavailable,
                },
                Err(error) => return shell_error(error),
            };
            if w <= 0 || h <= 0 {
                return PortalOutcome::Cancelled;
            }
            let shot = conn
                .call_method(
                    Some(SHELL),
                    PATH,
                    Some(SHELL),
                    "ScreenshotArea",
                    &(x, y, w, h, true, target_str.as_str()),
                )
                .await;
            match shot.map(|reply| reply.body().deserialize::<(bool, String)>()) {
                Ok(Ok((true, saved))) => {
                    let saved = PathBuf::from(saved);
                    if file_has_content(&saved) {
                        PortalOutcome::Saved(saved)
                    } else {
                        PortalOutcome::Unavailable
                    }
                }
                Ok(_) => PortalOutcome::Unavailable,
                Err(error) => shell_error(error),
            }
        }
        .await;
        if claimed {
            let _ = conn.release_name(ALLOWED_NAME).await;
        }
        outcome
    })
}

/// Refusals and missing methods fall through to the next method; anything else
/// (Escape during selection reports a cancelled operation) is a cancellation.
fn shell_error(error: zbus::Error) -> PortalOutcome {
    let name = match &error {
        zbus::Error::MethodError(name, _, _) => name.as_str().to_string(),
        _ => String::new(),
    };
    tracing::info!("GNOME Shell screenshot: {error}");
    if name.is_empty()
        || name.ends_with("AccessDenied")
        || name.ends_with("UnknownMethod")
        || name.ends_with("UnknownInterface")
        || name.ends_with("ServiceUnknown")
        || name.ends_with("UnknownObject")
    {
        PortalOutcome::Unavailable
    } else {
        PortalOutcome::Cancelled
    }
}

/// `None` when no supported tool is installed, otherwise whether one produced a file.
fn tool_capture(target: &Path) -> Option<bool> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let on_path = shelldeck_core::util::executable_on_path;
    let mut tried = false;

    // grim needs wlr-screencopy (sway, Hyprland…); it fails cleanly elsewhere.
    if wayland && on_path("grim") && on_path("slurp") {
        tried = true;
        if let Some(geometry) = Command::new("slurp")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|g| !g.is_empty())
        {
            let ok = Command::new("grim")
                .args(["-g", &geometry])
                .arg(target)
                .status()
                .is_ok_and(|s| s.success());
            if ok && file_has_content(target) {
                return Some(true);
            }
        } else {
            // slurp exits non-zero on Escape: a real cancellation.
            return Some(false);
        }
    }

    let attempts: &[(&str, &[&str])] = &[
        ("gnome-screenshot", &["-a", "-f"]),
        ("spectacle", &["-r", "-b", "-n", "-o"]),
        ("import", &[]),
    ];
    for (program, args) in attempts {
        if !on_path(program) {
            continue;
        }
        tried = true;
        let ok = Command::new(program)
            .args(*args)
            .arg(target)
            .status()
            .is_ok_and(|s| s.success());
        if ok && file_has_content(target) {
            return Some(true);
        }
    }
    tried.then_some(false)
}

fn system_key_capture() -> LinuxCapture {
    let dirs = screenshot_dirs();
    let started = SystemTime::now();
    let before = image_files(&dirs);
    if let Some(hint) = HINT_SINK.get() {
        hint();
    }
    let deadline = Instant::now() + WATCH_TIMEOUT;
    let mut candidate: Option<(PathBuf, u64)> = None;
    while Instant::now() < deadline {
        std::thread::sleep(WATCH_POLL);
        let found = new_image(&before, &image_files(&dirs), started);
        match (found, candidate.take()) {
            // Same size on two polls: the desktop finished writing it.
            (Some((path, size)), Some((prev, prev_size))) if path == prev && size == prev_size => {
                return LinuxCapture::Captured(path);
            }
            (Some(next), _) => candidate = Some(next),
            (None, _) => {}
        }
    }
    LinuxCapture::Cancelled
}

/// Pictures (from `user-dirs.dirs`, localized names included) and its
/// immediate subfolders, where desktops save screenshots.
fn screenshot_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let pictures = std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| pictures_dir_from_user_dirs(&text, &home))
        .unwrap_or_else(|| home.join("Pictures"));
    let mut dirs = vec![pictures.clone()];
    if let Ok(entries) = std::fs::read_dir(&pictures) {
        dirs.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    dirs
}

pub(crate) fn pictures_dir_from_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("XDG_PICTURES_DIR="))?;
    let value = line["XDG_PICTURES_DIR=".len()..].trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    (path != home && path.is_absolute()).then_some(path)
}

fn image_files(dirs: &[PathBuf]) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let image = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"));
            if !image {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    out.push((
                        path,
                        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        meta.len(),
                    ));
                }
            }
        }
    }
    out
}

/// The newest non-empty image that was not there before and was written after `since`.
pub(crate) fn new_image(
    before: &[(PathBuf, SystemTime, u64)],
    now: &[(PathBuf, SystemTime, u64)],
    since: SystemTime,
) -> Option<(PathBuf, u64)> {
    let known: HashSet<&PathBuf> = before.iter().map(|(p, _, _)| p).collect();
    now.iter()
        .filter(|(path, modified, size)| !known.contains(path) && *size > 0 && *modified >= since)
        .max_by_key(|(_, modified, _)| *modified)
        .map(|(path, _, size)| (path.clone(), *size))
}

fn file_has_content(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.len() > 0)
}

#[cfg(test)]
mod tests {
    use super::{new_image, pictures_dir_from_user_dirs};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    // SDTEST-2904
    #[test]
    fn localized_pictures_dir_is_read_from_user_dirs() {
        let home = Path::new("/home/ana");
        let text =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Bureau\"\nXDG_PICTURES_DIR=\"$HOME/Images\"\n";
        assert_eq!(
            pictures_dir_from_user_dirs(text, home),
            Some(PathBuf::from("/home/ana/Images"))
        );
        // A disabled entry points at $HOME itself: never watch the whole home.
        assert_eq!(
            pictures_dir_from_user_dirs("XDG_PICTURES_DIR=\"$HOME/\"", home),
            None
        );
        assert_eq!(pictures_dir_from_user_dirs("", home), None);
    }

    // SDTEST-2905
    #[test]
    fn only_a_fresh_non_empty_screenshot_is_picked_up() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let old = (
            PathBuf::from("/p/old.png"),
            t0 - Duration::from_secs(60),
            10,
        );
        let before = vec![old.clone()];
        let copied_in = (
            PathBuf::from("/p/copied.png"),
            t0 - Duration::from_secs(5),
            10,
        );
        let empty = (
            PathBuf::from("/p/Captures/empty.png"),
            t0 + Duration::from_secs(1),
            0,
        );
        let shot = (
            PathBuf::from("/p/Captures/shot.png"),
            t0 + Duration::from_secs(2),
            42,
        );
        let now = vec![old, copied_in, empty, shot];
        assert_eq!(
            new_image(&before, &now, t0),
            Some((PathBuf::from("/p/Captures/shot.png"), 42))
        );
        assert_eq!(new_image(&before, &before, t0), None);
    }
}
