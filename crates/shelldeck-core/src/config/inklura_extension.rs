//! Local setup of the Inklura browser assistant (Chrome extension).
//!
//! Inklura Manage serves the extension as a source-only package at
//! `{manage}/extensions/inklura-assistant.zip` (built by
//! `bext/extensions/inklura-assistant/package.py`). The package holds:
//!
//! - `extension/` — the unpacked MV3 extension Chrome loads;
//! - `native/` — the optional Craft MCP native-messaging bridge and its
//!   per-user installer (`native/install.py`, macOS/Linux only).
//!
//! Chrome does not let a third-party program install an extension that is
//! not on the Web Store, so ShellDeck does everything around the one step
//! only the user may take ("Load unpacked"):
//!
//! 1. download and unpack the package into a **stable** folder under the
//!    ShellDeck data directory — an unpacked extension's ID is derived from
//!    its absolute path, so updates replace the files in place and Chrome
//!    keeps the same ID, permissions and pinned icon;
//! 2. open `chrome://extensions` in Chrome (a URL that `xdg-open`/`open`/
//!    `start` refuse, so Chrome itself is launched with it);
//! 3. compute the extension ID ShellDeck's folder will receive, so the Craft
//!    bridge can be installed without the user copying the ID by hand.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::{Result, ShellDeckError};

/// Path of the package on the Manage origin.
pub const PACKAGE_PATH: &str = "/extensions/inklura-assistant.zip";
/// Name Manage gives the native-messaging host; used to report bridge state.
pub const NATIVE_HOST_NAME: &str = "fr.inklura.craft";
/// Folder (under the ShellDeck data dir) the package is unpacked into.
const INSTALL_FOLDER: &str = "inklura-assistant";
/// Refuse absurd downloads: the package is ~13 KB today.
const MAX_PACKAGE_BYTES: usize = 16 * 1024 * 1024;
/// Refuse absurd entries inside it (zip bombs).
const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

/// The extension as it is currently unpacked on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtension {
    /// Package root (contains `extension/` and `native/`).
    pub root: PathBuf,
    /// Folder to pick in Chrome's "Load unpacked" dialog.
    pub extension_dir: PathBuf,
    /// `version` from the extension manifest.
    pub version: String,
    /// The ID Chrome assigns when `extension_dir` is loaded unpacked.
    pub extension_id: String,
}

/// Where ShellDeck keeps the unpacked package.
pub fn install_root() -> PathBuf {
    match directories::ProjectDirs::from("com", "shelldeck", "ShellDeck") {
        Some(dirs) => dirs.data_dir().join(INSTALL_FOLDER),
        None => crate::config::AppConfig::config_dir().join(INSTALL_FOLDER),
    }
}

/// Package URL for a Manage origin (`https://manage.inklura.fr`).
pub fn package_url(manage_base_url: &str) -> String {
    format!("{}{}", manage_base_url.trim_end_matches('/'), PACKAGE_PATH)
}

/// Read the unpacked extension under `root`, if a valid one is there.
pub fn installed_at(root: &Path) -> Option<InstalledExtension> {
    let extension_dir = root.join("extension");
    let manifest = std::fs::read_to_string(extension_dir.join("manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let version = manifest.get("version")?.as_str()?.to_string();
    let extension_id = unpacked_extension_id(&chrome_load_path(&extension_dir));
    Some(InstalledExtension {
        root: root.to_path_buf(),
        extension_dir,
        version,
        extension_id,
    })
}

/// [`installed_at`] for the default [`install_root`].
pub fn installed() -> Option<InstalledExtension> {
    installed_at(&install_root())
}

/// Download the package from Manage and unpack it into `root`, replacing
/// any previous copy in place (same path ⇒ same Chrome extension ID).
pub fn install_from(manage_base_url: &str, root: &Path) -> Result<InstalledExtension> {
    let url = package_url(manage_base_url);
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {e}")))?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("download failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(ShellDeckError::Connection(format!(
            "download failed: {url} returned HTTP {}",
            status.as_u16()
        )));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| ShellDeckError::Connection(format!("download failed: {e}")))?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(ShellDeckError::Config(
            "extension package is too large".into(),
        ));
    }
    install_package_bytes(&bytes, root)
}

/// [`install_from`] into the default [`install_root`].
pub fn install(manage_base_url: &str) -> Result<InstalledExtension> {
    install_from(manage_base_url, &install_root())
}

/// Unpack `package` into `root`. The archive is fully extracted and validated
/// in a sibling staging folder first, so a corrupt download never destroys a
/// working install.
pub fn install_package_bytes(package: &[u8], root: &Path) -> Result<InstalledExtension> {
    let parent = root
        .parent()
        .ok_or_else(|| ShellDeckError::Config("invalid install folder".into()))?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".{INSTALL_FOLDER}-staging"));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let staged = extract_zip(package, &staging).and_then(|()| {
        installed_at(&staging).ok_or_else(|| {
            ShellDeckError::Config("package does not contain extension/manifest.json".into())
        })
    });
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    std::fs::rename(&staging, root)?;
    installed_at(root)
        .ok_or_else(|| ShellDeckError::Config("installed extension is unreadable".into()))
}

/// The path string Chrome hashes when the user picks `dir` in "Load
/// unpacked": Chrome makes it absolute and resolves symlinks.
fn chrome_load_path(dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        // `canonicalize` adds a `\\?\` prefix Chrome never sees.
        std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
    }
    #[cfg(not(windows))]
    {
        dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
    }
}

/// ID Chrome assigns an unpacked extension loaded from `path` (no `key` in
/// its manifest): SHA-256 of the path, first 16 bytes, hex digits mapped
/// `0-f` → `a-p`. POSIX hashes the path bytes; Windows hashes the UTF-16LE
/// path with an upper-case drive letter (`crx_file::id_util`).
pub fn unpacked_extension_id(path: &Path) -> String {
    #[cfg(windows)]
    let bytes: Vec<u8> = {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.len() >= 2 && wide[1] == u16::from(b':') && wide[0] < 128 {
            wide[0] = u16::from((wide[0] as u8).to_ascii_uppercase());
        }
        wide.iter().flat_map(|u| u.to_le_bytes()).collect()
    };
    #[cfg(not(windows))]
    let bytes: Vec<u8> = path.as_os_str().as_encoded_bytes().to_vec();
    let digest = Sha256::digest(&bytes);
    digest[..16]
        .iter()
        .flat_map(|b| [b >> 4, b & 0x0f])
        .map(|nibble| char::from(b'a' + nibble))
        .collect()
}

/// Minimal ZIP reader for the package (stored + deflate, CRC-checked).
/// Rejects absolute paths and `..` so an entry can never escape `dest`.
fn extract_zip(data: &[u8], dest: &Path) -> Result<()> {
    let bad = |msg: &str| ShellDeckError::Config(format!("invalid extension package: {msg}"));
    let u16_at = |at: usize| -> Result<usize> {
        data.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
            .ok_or_else(|| bad("truncated"))
    };
    let u32_at = |at: usize| -> Result<usize> {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| bad("truncated"))
    };

    // End of central directory: last signature within the 64 KiB comment window.
    let search_from = data.len().saturating_sub(22 + 0xffff);
    let eocd = (search_from..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| bad("not a zip archive"))?;
    let entries = u16_at(eocd + 10)?;
    let mut at = u32_at(eocd + 16)?;

    for _ in 0..entries {
        if u32_at(at)? != 0x0201_4b50 {
            return Err(bad("corrupt central directory"));
        }
        let method = u16_at(at + 10)?;
        let crc = u32_at(at + 16)? as u32;
        let compressed = u32_at(at + 20)?;
        let size = u32_at(at + 24)? as u64;
        let name_len = u16_at(at + 28)?;
        let extra_len = u16_at(at + 30)?;
        let comment_len = u16_at(at + 32)?;
        let local = u32_at(at + 42)?;
        let name = data
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| bad("truncated"))?;
        let name = std::str::from_utf8(name).map_err(|_| bad("non-UTF-8 entry name"))?;
        at += 46 + name_len + extra_len + comment_len;

        let relative = Path::new(name);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(bad("unsafe entry path"));
        }
        let target = dest.join(relative);
        if name.ends_with('/') {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if size > MAX_ENTRY_BYTES {
            return Err(bad("entry too large"));
        }

        if u32_at(local)? != 0x0403_4b50 {
            return Err(bad("corrupt local header"));
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let raw = data
            .get(start..start + compressed)
            .ok_or_else(|| bad("truncated"))?;
        let contents = match method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(size as usize);
                flate2::read::DeflateDecoder::new(raw)
                    .take(MAX_ENTRY_BYTES + 1)
                    .read_to_end(&mut out)
                    .map_err(|_| bad("corrupt deflate stream"))?;
                out
            }
            _ => return Err(bad("unsupported compression")),
        };
        if contents.len() as u64 != size || crc32fast::hash(&contents) != crc {
            return Err(bad("checksum mismatch"));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, contents)?;
        #[cfg(unix)]
        if relative.extension().is_some_and(|e| e == "py") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// One installed Chrome flavour: its executable and its user-data folder
/// (where `Local State` lists the profiles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeInstall {
    pub binary: PathBuf,
    pub user_data_dir: PathBuf,
}

/// A Chrome profile, resolved from `Local State`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeProfile {
    /// Folder name passed to `--profile-directory` (`Default`, `Profile 1`…).
    pub directory: String,
    /// Display name Chrome shows for the profile.
    pub name: String,
    /// Google account signed into the profile, if any.
    pub email: String,
    /// The extension is registered in this profile's preferences.
    pub extension_loaded: bool,
}

/// Installed Chrome flavours, Google Chrome first, then Chromium.
fn chrome_installs() -> Vec<ChromeInstall> {
    #[cfg(target_os = "linux")]
    let pairs: Vec<(Vec<PathBuf>, Option<PathBuf>)> = {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let find = |names: &[&str]| -> Vec<PathBuf> {
            std::env::split_paths(&path)
                .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
                .collect()
        };
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| crate::util::home_dir().map(|h| h.join(".config")));
        vec![
            (
                find(&["google-chrome", "google-chrome-stable"]),
                config.as_ref().map(|c| c.join("google-chrome")),
            ),
            (
                find(&["google-chrome-beta"]),
                config.as_ref().map(|c| c.join("google-chrome-beta")),
            ),
            (
                find(&["chromium", "chromium-browser"]),
                config.as_ref().map(|c| c.join("chromium")),
            ),
        ]
    };
    #[cfg(target_os = "macos")]
    let pairs: Vec<(Vec<PathBuf>, Option<PathBuf>)> = {
        let home = crate::util::home_dir();
        let apps = |bundle: &str| -> Vec<PathBuf> {
            let mut roots = vec![PathBuf::from("/Applications")];
            if let Some(h) = &home {
                roots.push(h.join("Applications"));
            }
            roots.into_iter().map(|r| r.join(bundle)).collect()
        };
        let support = home.as_ref().map(|h| h.join("Library/Application Support"));
        vec![
            (
                apps("Google Chrome.app/Contents/MacOS/Google Chrome"),
                support.as_ref().map(|s| s.join("Google/Chrome")),
            ),
            (
                apps("Chromium.app/Contents/MacOS/Chromium"),
                support.as_ref().map(|s| s.join("Chromium")),
            ),
        ]
    };
    #[cfg(target_os = "windows")]
    let pairs: Vec<(Vec<PathBuf>, Option<PathBuf>)> = {
        let binaries = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|root| PathBuf::from(root).join(r"Google\Chrome\Application\chrome.exe"))
            .collect();
        let data = std::env::var_os("LOCALAPPDATA")
            .map(|l| PathBuf::from(l).join(r"Google\Chrome\User Data"));
        vec![(binaries, data)]
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let pairs: Vec<(Vec<PathBuf>, Option<PathBuf>)> = Vec::new();

    pairs
        .into_iter()
        .filter_map(|(binaries, data)| {
            let binary = binaries.into_iter().find(|p| p.is_file())?;
            Some(ChromeInstall {
                binary,
                user_data_dir: data?,
            })
        })
        .collect()
}

/// Whether a Chrome/Chromium executable was found.
pub fn chrome_available() -> bool {
    !chrome_installs().is_empty()
}

/// Profiles listed in a `Local State` document, `last_used` first.
fn profiles_from_local_state(local_state: &str) -> Vec<(String, String, String)> {
    let Ok(state) = serde_json::from_str::<serde_json::Value>(local_state) else {
        return Vec::new();
    };
    let last_used = state
        .pointer("/profile/last_used")
        .and_then(|v| v.as_str())
        .unwrap_or("Default")
        .to_string();
    let mut profiles: Vec<(String, String, String)> = state
        .pointer("/profile/info_cache")
        .and_then(|v| v.as_object())
        .map(|cache| {
            cache
                .iter()
                .map(|(dir, info)| {
                    let field = |k: &str| {
                        info.get(k)
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string()
                    };
                    (dir.clone(), field("name"), field("user_name"))
                })
                .collect()
        })
        .unwrap_or_default();
    profiles.sort_by_key(|(dir, _, _)| (dir != &last_used, dir.clone()));
    profiles
}

/// Pick the profile signed in with `email` (case-insensitive), else the
/// last-used one. Returns `(directory, name, email)`.
fn pick_profile(local_state: &str, email: Option<&str>) -> Option<(String, String, String)> {
    let profiles = profiles_from_local_state(local_state);
    let wanted = email.map(str::trim).filter(|e| !e.is_empty());
    wanted
        .and_then(|e| {
            profiles
                .iter()
                .find(|(_, _, user)| user.eq_ignore_ascii_case(e))
                .cloned()
        })
        .or_else(|| profiles.into_iter().next())
}

/// Whether a profile's preferences register `extension_id`.
fn profile_has_extension(profile_dir: &Path, extension_id: &str) -> bool {
    ["Preferences", "Secure Preferences"].iter().any(|file| {
        std::fs::read_to_string(profile_dir.join(file))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|prefs| {
                prefs
                    .pointer("/extensions/settings")
                    .and_then(|s| s.as_object())
                    .map(|s| s.contains_key(extension_id))
            })
            .unwrap_or(false)
    })
}

/// The Chrome install and profile ShellDeck targets for `email` (the
/// signed-in Manage account): the profile signed into the same Google
/// account, otherwise the last-used profile.
pub fn target_profile(
    email: Option<&str>,
    extension_id: Option<&str>,
) -> Option<(ChromeInstall, Option<ChromeProfile>)> {
    let installs = chrome_installs();
    let read =
        |i: &ChromeInstall| std::fs::read_to_string(i.user_data_dir.join("Local State")).ok();
    // Prefer the flavour that actually has a profile for this account.
    let matched = email.and_then(|e| {
        installs.iter().find_map(|install| {
            let state = read(install)?;
            let picked = pick_profile(&state, Some(e))?;
            picked
                .2
                .eq_ignore_ascii_case(e.trim())
                .then(|| (install.clone(), picked))
        })
    });
    let (install, picked) = match matched {
        Some((install, picked)) => (install, Some(picked)),
        None => {
            let install = installs.into_iter().next()?;
            let picked = read(&install).and_then(|s| pick_profile(&s, None));
            (install, picked)
        }
    };
    let profile = picked.map(|(directory, name, email)| ChromeProfile {
        extension_loaded: extension_id
            .is_some_and(|id| profile_has_extension(&install.user_data_dir.join(&directory), id)),
        directory,
        name,
        email,
    });
    Some((install, profile))
}

/// Open `chrome://extensions` in the Chrome profile used by `email`. A
/// running Chrome receives it as a new tab in that profile's window.
pub fn open_extensions_page(email: Option<&str>) -> Result<Option<ChromeProfile>> {
    let (install, profile) = target_profile(email, None)
        .ok_or_else(|| ShellDeckError::NotFound("Google Chrome".into()))?;
    let mut cmd = Command::new(&install.binary);
    if let Some(p) = &profile {
        cmd.arg(format!("--profile-directory={}", p.directory));
    }
    cmd.arg("chrome://extensions")
        .spawn()
        .map(|_| profile)
        .map_err(|e| ShellDeckError::Connection(format!("failed to start Chrome: {e}")))
}

/// Whether the Craft bridge installer supports this OS (`native/install.py`
/// refuses Windows).
pub const fn native_bridge_supported() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos"))
}

/// Arguments for the package's own native-bridge installer.
fn native_bridge_args(ext: &InstalledExtension, workspace: &Path) -> Vec<std::ffi::OsString> {
    vec![
        ext.root.join("native").join("install.py").into_os_string(),
        "--extension-id".into(),
        ext.extension_id.clone().into(),
        "--workspace".into(),
        workspace.as_os_str().to_owned(),
    ]
}

/// Install the Craft MCP native-messaging bridge for this user by running
/// the package's installer with the computed extension ID. Returns the
/// installer's report (found Craft apps, workspace).
pub fn install_native_bridge(ext: &InstalledExtension, workspace: &Path) -> Result<String> {
    if !native_bridge_supported() {
        return Err(ShellDeckError::Config(
            "the Craft bridge is available on macOS and Linux only".into(),
        ));
    }
    let output = Command::new("python3")
        .args(native_bridge_args(ext, workspace))
        .output()
        .map_err(|e| ShellDeckError::NotFound(format!("python3 ({e})")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rev().find(|l| !l.trim().is_empty());
        Err(ShellDeckError::Config(
            last.unwrap_or("native bridge installer failed")
                .trim()
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// Build a deflate ZIP in memory (the reader's contract is what Python's
    /// `zipfile` emits for `package.py`: local headers + central directory).
    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in entries {
            let mut enc =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(body).unwrap();
            let packed = enc.finish().unwrap();
            let crc = crc32fast::hash(body);
            let offset = out.len() as u32;
            let header = |sig: u32, central: bool| {
                let mut h = Vec::new();
                h.extend(sig.to_le_bytes());
                if central {
                    h.extend(20u16.to_le_bytes());
                }
                h.extend(20u16.to_le_bytes()); // version needed
                h.extend(0u16.to_le_bytes()); // flags
                h.extend(8u16.to_le_bytes()); // deflate
                h.extend(0u32.to_le_bytes()); // time/date
                h.extend(crc.to_le_bytes());
                h.extend((packed.len() as u32).to_le_bytes());
                h.extend((body.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes()); // extra
                if central {
                    h.extend([0u8; 6]); // comment len, disk, internal attrs
                    h.extend(0u32.to_le_bytes()); // external attrs
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(header(0x0403_4b50, false));
            out.extend(&packed);
            central.extend(header(0x0201_4b50, true));
        }
        let cd_offset = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    const MANIFEST: &[u8] = br#"{"manifest_version":3,"name":"Inklura","version":"0.1.0"}"#;

    // SDTEST-2875 — vector taken from a real Chrome profile on Linux
    // (Preferences → extensions.settings, location 4 = unpacked).
    #[cfg(not(windows))]
    #[test]
    fn sdtest_2875_unpacked_id_matches_chrome() {
        assert_eq!(
            unpacked_extension_id(Path::new(
                "/home/wd29-pc/dev/company-manager-new/packages/chrome-extension/dist"
            )),
            "coclcblpabicamklkjabojgpfndihkif"
        );
    }

    // SDTEST-2876 — the downloaded package is fetched from the Manage route,
    // unpacked, and an update replaces files in place so the ID is stable.
    #[test]
    fn sdtest_2876_install_downloads_unpacks_and_updates_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inklura-assistant");
        let package = zip(&[
            ("extension/manifest.json", MANIFEST),
            ("native/install.py", b"print('ok')"),
        ]);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let mut resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/zip\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                package.len()
            )
            .into_bytes();
            resp.extend(&package);
            stream.write_all(&resp).unwrap();
            request
        });

        // A previous install with a stale file must not survive the update.
        std::fs::create_dir_all(root.join("extension")).unwrap();
        std::fs::write(root.join("extension/stale.js"), "old").unwrap();
        let before_id = unpacked_extension_id(&chrome_load_path(&root.join("extension")));

        let installed = install_from(&format!("{base}/"), &root).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /extensions/inklura-assistant.zip "));
        assert_eq!(installed.version, "0.1.0");
        assert_eq!(installed.extension_id, before_id);
        assert!(!root.join("extension/stale.js").exists());
        assert!(root.join("native/install.py").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(root.join("native/install.py"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert_eq!(installed_at(&root), Some(installed));
    }

    // SDTEST-2877 — a hostile or broken package leaves the working install intact.
    #[test]
    fn sdtest_2877_rejects_escaping_or_invalid_packages_without_touching_install() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inklura-assistant");
        install_package_bytes(&zip(&[("extension/manifest.json", MANIFEST)]), &root).unwrap();

        for package in [
            zip(&[
                ("../escape.txt", b"x"),
                ("extension/manifest.json", MANIFEST),
            ]),
            zip(&[("native/install.py", b"no manifest")]),
            b"<html>login</html>".to_vec(),
        ] {
            assert!(install_package_bytes(&package, &root).is_err());
        }
        assert!(!dir.path().join("escape.txt").exists());
        assert_eq!(installed_at(&root).unwrap().version, "0.1.0");
    }

    // SDTEST-2879 — the profile signed into the Manage account wins over the
    // last-used one; without a match, the last-used profile is targeted.
    #[test]
    fn sdtest_2879_picks_chrome_profile_for_account() {
        let state = r#"{"profile":{"last_used":"Profile 1","info_cache":{
            "Default":{"name":"Personne 1","user_name":"ben@webdesign29.net"},
            "Profile 1":{"name":"Benjamin","user_name":"benfavre@gmail.com"},
            "Profile 3":{"name":"jordan","user_name":"jordan@webdesign29.net"}}}}"#;
        let dir = |email| pick_profile(state, email).map(|p| p.0);
        assert_eq!(dir(Some("Ben@WebDesign29.net")), Some("Default".into()));
        assert_eq!(dir(Some("nobody@example.com")), Some("Profile 1".into()));
        assert_eq!(dir(None), Some("Profile 1".into()));
        assert_eq!(pick_profile("not json", Some("ben@webdesign29.net")), None);

        let profiles = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(profiles.path().join("Default")).unwrap();
        std::fs::write(
            profiles.path().join("Default/Preferences"),
            r#"{"extensions":{"settings":{"lkidjmdekjibhkgigkjdhfgbiccgpgmn":{"location":4}}}}"#,
        )
        .unwrap();
        let default = profiles.path().join("Default");
        assert!(profile_has_extension(
            &default,
            "lkidjmdekjibhkgigkjdhfgbiccgpgmn"
        ));
        assert!(!profile_has_extension(
            &default,
            "coclcblpabicamklkjabojgpfndihkif"
        ));
    }

    // SDTEST-2878 — the bridge installer receives the computed ID, never a placeholder.
    #[test]
    fn sdtest_2878_native_bridge_args_carry_computed_id() {
        let ext = InstalledExtension {
            root: PathBuf::from("/data/inklura-assistant"),
            extension_dir: PathBuf::from("/data/inklura-assistant/extension"),
            version: "0.1.0".into(),
            extension_id: unpacked_extension_id(Path::new("/data/inklura-assistant/extension")),
        };
        let args = native_bridge_args(&ext, Path::new("/home/me/Documents"));
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[1], "--extension-id");
        assert_eq!(args[2].len(), 32);
        assert!(args[2].chars().all(|c| ('a'..='p').contains(&c)));
        assert_eq!(args[4], "/home/me/Documents");
        assert!(args[0].ends_with("install.py"));
    }
}

#[cfg(test)]
mod live_tests {
    // Live smoke: real package from Manage (SHELLDECK_LIVE=1, never in CI).
    #[test]
    #[ignore = "network: SHELLDECK_LIVE=1 hits manage.inklura.fr"]
    fn live_install_real_manage_package() {
        let dir = tempfile::tempdir().unwrap();
        let ext = super::install_from("https://manage.inklura.fr", &dir.path().join("x")).unwrap();
        assert!(ext.root.join("native/install.py").is_file());
        eprintln!("{ext:?}");
    }
}
