//! "Share with Inklura" launch intent.
//!
//! File managers hand files to Inklura Desktop through process arguments
//! (`shelldeck --share <file>…` from the Linux "Open with" entry and the
//! Windows "Send to" shortcut) or, on macOS, through Launch Services
//! `file://` URLs. Either way the files only get **staged** in User mode →
//! Share: the user still reviews them and clicks Share. Nothing is uploaded
//! by the intent itself.
//!
//! The intent travels through the single-instance hand-off as one line, so it
//! is encoded as `inklura-share:` + a JSON array of absolute UTF-8 paths. That
//! prefix is deliberately **not** a `shelldeck://` URL: a web page can open
//! `shelldeck://…` links, but only a local process can put this payload on
//! the authenticated loopback channel. Browser-originated links therefore can
//! never pre-stage arbitrary local files.
//!
//! This module is pure (std + serde_json + url) so the grammar is testable
//! without a running app.

use std::path::{Path, PathBuf};

/// Payload prefix on the single-instance channel.
pub const PREFIX: &str = "inklura-share:";

/// Upper bound on forwarded paths. The Share view enforces its own (smaller)
/// batch limit and explains it; this only bounds the IPC line.
pub const MAX_FORWARDED_FILES: usize = 100;

/// Files a local launch asked Inklura Desktop to stage for sharing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareIntent {
    pub files: Vec<PathBuf>,
}

impl ShareIntent {
    /// Encode as a single-line hand-off payload. Returns `None` when there is
    /// nothing to share or a path is not valid UTF-8 / not absolute.
    pub fn encode(&self) -> Option<String> {
        if self.files.is_empty() || self.files.len() > MAX_FORWARDED_FILES {
            return None;
        }
        let mut list = Vec::with_capacity(self.files.len());
        for path in &self.files {
            if !path.is_absolute() {
                return None;
            }
            list.push(path.to_str()?.to_owned());
        }
        // serde_json escapes control characters, so the payload stays on one
        // line even for file names that contain newlines.
        Some(format!("{PREFIX}{}", serde_json::to_string(&list).ok()?))
    }

    /// Decode a hand-off payload. Relative paths, empty lists and malformed
    /// JSON are rejected as a whole rather than partially applied.
    pub fn decode(payload: &str) -> Option<Self> {
        let json = payload.trim().strip_prefix(PREFIX)?;
        let list: Vec<String> = serde_json::from_str(json).ok()?;
        if list.is_empty() || list.len() > MAX_FORWARDED_FILES {
            return None;
        }
        let files: Vec<PathBuf> = list.into_iter().map(PathBuf::from).collect();
        if files.iter().any(|path| !path.is_absolute()) {
            return None;
        }
        Some(Self { files })
    }

    /// Build an intent from macOS Launch Services URLs. Only local `file://`
    /// URLs are accepted; anything else returns `None` so the caller keeps
    /// routing it as a deep link.
    pub fn from_file_urls<'a>(urls: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        let mut files = Vec::new();
        for raw in urls {
            let parsed = url::Url::parse(raw.trim()).ok()?;
            if parsed.scheme() != "file" {
                return None;
            }
            if parsed
                .host_str()
                .is_some_and(|host| !host.is_empty() && !host.eq_ignore_ascii_case("localhost"))
            {
                return None;
            }
            files.push(parsed.to_file_path().ok()?);
        }
        if files.is_empty() || files.len() > MAX_FORWARDED_FILES {
            return None;
        }
        Some(Self { files })
    }
}

/// True for the `file://` URLs macOS delivers when files are opened with the
/// app (Finder "Open With", drop on the Dock icon).
pub fn is_file_url(raw: &str) -> bool {
    raw.trim()
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file://"))
}

/// Resolve CLI paths against `cwd` and keep only existing regular files.
/// Returns the absolute paths, or the first argument that is not a file.
pub fn resolve_cli_paths(args: &[String], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::with_capacity(args.len());
    for arg in args {
        let candidate = Path::new(arg);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        match std::fs::metadata(&absolute) {
            Ok(meta) if meta.is_file() => files.push(absolute),
            _ => return Err(arg.clone()),
        }
    }
    Ok(files)
}

/// File name of the per-user Linux "Share with Inklura" entry.
pub const LINUX_SHARE_DESKTOP_FILE: &str = "inklura-desktop-share.desktop";

/// MIME types Share accepts (mirrors `share::mime_for`), advertised so file
/// managers list "Share with Inklura" under "Open With".
pub const SHARE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/avif",
    "image/bmp",
    "image/x-icon",
    "application/pdf",
    "video/mp4",
    "video/webm",
    "video/quicktime",
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "text/plain",
    "text/markdown",
    "text/csv",
    "application/json",
    "text/x-log",
    "application/zip",
    "application/gzip",
    "application/x-7z-compressed",
];

/// Quote one Exec argument per the Desktop Entry spec.
fn desktop_exec_quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for ch in arg.chars() {
        if matches!(ch, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    // `%` is a field code prefix even inside quotes.
    out.replace('%', "%%")
}

/// The hidden "Open With" entry. It never becomes a default handler and is
/// not shown in launchers (`NoDisplay`), it only appears in file managers.
pub fn linux_share_desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
Type=Application\n\
Name=Share with Inklura\n\
Name[fr]=Partager avec Inklura\n\
Comment=Prepare these files in Inklura Desktop Share\n\
Comment[fr]=Préparer ces fichiers dans Inklura Desktop Partage\n\
Exec={} --share %F\n\
Icon=shelldeck\n\
Terminal=false\n\
NoDisplay=true\n\
StartupWMClass=shelldeck\n\
MimeType={};\n",
        desktop_exec_quote(exe),
        SHARE_MIME_TYPES.join(";")
    )
}

/// Write (or refresh) the per-user Linux share entry for the running binary.
/// AppImages register their stable `$APPIMAGE` path, not the transient mount.
/// Returns `true` when the file changed. Best-effort: callers log failures.
#[cfg(target_os = "linux")]
pub fn install_linux_share_entry() -> std::io::Result<bool> {
    let exe = match std::env::var_os("APPIMAGE") {
        Some(appimage) if !appimage.is_empty() => PathBuf::from(appimage),
        _ => std::env::current_exe()?,
    };
    let exe = exe.to_str().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "non-UTF-8 executable path")
    })?;
    let Some(dirs) = directories::BaseDirs::new() else {
        return Ok(false);
    };
    let apps = dirs.data_dir().join("applications");
    let path = apps.join(LINUX_SHARE_DESKTOP_FILE);
    let entry = linux_share_desktop_entry(exe);
    if std::fs::read_to_string(&path).is_ok_and(|current| current == entry) {
        return Ok(false);
    }
    std::fs::create_dir_all(&apps)?;
    let staging = apps.join(format!(".{LINUX_SHARE_DESKTOP_FILE}.tmp"));
    std::fs::write(&staging, entry)?;
    std::fs::rename(&staging, &path)?;
    // Refresh the MIME cache so "Open With" lists the entry right away.
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    // The payload is one line, round-trips exactly (including awkward file
    // names) and is never mistaken for a browser-reachable deep link.
    #[test]
    fn encodes_single_line_and_round_trips() {
        let intent = ShareIntent {
            files: vec![abs("capture 1.png"), abs("line\nbreak.pdf"), abs("é€.txt")],
        };
        let payload = intent.encode().expect("absolute utf-8 paths encode");
        assert!(payload.starts_with(PREFIX));
        assert!(!payload.contains('\n'));
        assert!(!crate::config::deep_link::DeepLink::looks_like(&payload));
        assert_eq!(ShareIntent::decode(&payload), Some(intent));
    }

    #[test]
    fn rejects_relative_empty_and_malformed_payloads() {
        assert_eq!(ShareIntent { files: vec![] }.encode(), None);
        assert_eq!(
            ShareIntent {
                files: vec![PathBuf::from("relative.png")]
            }
            .encode(),
            None
        );
        assert_eq!(ShareIntent::decode("inklura-share:[]"), None);
        assert_eq!(
            ShareIntent::decode("inklura-share:[\"relative.png\"]"),
            None
        );
        assert_eq!(ShareIntent::decode("inklura-share:{"), None);
        assert_eq!(
            ShareIntent::decode("shelldeck://share?file=/etc/passwd"),
            None
        );
        let too_many = format!(
            "{PREFIX}{}",
            serde_json::to_string(
                &(0..=MAX_FORWARDED_FILES)
                    .map(|i| abs(&format!("{i}")).to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        assert_eq!(ShareIntent::decode(&too_many), None);
    }

    #[test]
    fn accepts_only_local_file_urls() {
        let dir = std::env::temp_dir();
        let url = url::Url::from_file_path(dir.join("a b.png")).unwrap();
        let intent = ShareIntent::from_file_urls([url.as_str()]).expect("local file url");
        assert_eq!(intent.files, vec![dir.join("a b.png")]);
        assert!(is_file_url(url.as_str()));
        assert!(!is_file_url("shelldeck://agents"));
        assert_eq!(ShareIntent::from_file_urls(["shelldeck://agents"]), None);
        assert_eq!(
            ShareIntent::from_file_urls(["file://evil.example/share/x.png"]),
            None
        );
        assert_eq!(ShareIntent::from_file_urls(std::iter::empty()), None);
    }

    #[test]
    fn linux_entry_quotes_exec_and_lists_share_types() {
        let entry = linux_share_desktop_entry("/opt/My $Apps/100%/shelldeck");
        assert!(entry.contains("Exec=\"/opt/My \\$Apps/100%%/shelldeck\" --share %F\n"));
        assert!(entry.contains("NoDisplay=true\n"));
        assert!(entry.contains("MimeType=image/png;"));
        assert!(entry.contains("application/pdf;"));
        assert!(entry.ends_with(";\n"));
    }

    #[test]
    fn resolves_cli_paths_and_rejects_missing_or_directories() {
        let dir = std::env::temp_dir().join(format!("inklura-share-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.txt"), b"1").unwrap();
        let resolved = resolve_cli_paths(&["one.txt".into()], &dir).unwrap();
        assert_eq!(resolved, vec![dir.join("one.txt")]);
        assert_eq!(
            resolve_cli_paths(&["missing.txt".into()], &dir),
            Err("missing.txt".into())
        );
        let dir_arg = dir.to_string_lossy().into_owned();
        assert_eq!(
            resolve_cli_paths(std::slice::from_ref(&dir_arg), &dir),
            Err(dir_arg)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
