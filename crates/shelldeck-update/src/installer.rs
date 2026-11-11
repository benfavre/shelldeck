use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncWriteExt;

use crate::ReleaseInfo;

/// Download the release archive to a sibling temporary file, verifying the
/// signed metadata, byte count and SHA-256 digest before publishing `dest`.
pub async fn download_and_verify(release: &ReleaseInfo, dest: &Path) -> Result<()> {
    crate::verify_release_signature(release)?;
    download_verified_payload(release, dest).await
}

async fn download_verified_payload(release: &ReleaseInfo, dest: &Path) -> Result<()> {
    tracing::info!("Downloading update from {}", release.url);

    anyhow::ensure!(
        release.size > 0,
        "Update manifest has an empty payload size"
    );
    let mut response = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(10 * 60))
        .build()?
        .get(&release.url)
        .send()
        .await
        .context("Failed to download update")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Update download failed with HTTP {}",
            response.status().as_u16()
        );
    }

    let partial = tempfile::Builder::new()
        .prefix(".shelldeck-update-")
        .tempfile_in(
            dest.parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new(".")),
        )?
        .into_temp_path();
    let mut file = tokio::fs::File::create(&partial)
        .await
        .context("Failed to create temporary update file")?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read update payload")?
    {
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .context("Update payload size overflow")?;
        if downloaded > release.size {
            anyhow::bail!(
                "Update payload exceeds signed size: expected {} bytes",
                release.size
            );
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .context("Failed to write temporary update file")?;
    }
    file.flush().await?;
    drop(file);

    if downloaded != release.size {
        anyhow::bail!(
            "Update size mismatch: expected {}, got {}",
            release.size,
            downloaded
        );
    }

    let digest = format!("{:x}", hasher.finalize());

    if !digest.eq_ignore_ascii_case(&release.sha256) {
        anyhow::bail!(
            "SHA-256 mismatch: expected {}, got {}",
            release.sha256,
            digest
        );
    }

    tracing::info!("SHA-256 verified: {}", digest);
    tokio::fs::rename(&partial, dest)
        .await
        .context("Failed to publish verified update file")?;

    Ok(())
}

/// Platform-specific installation of the downloaded archive.
///
/// - **Linux**: extract tar.gz, stage beside the executable, then rename with rollback.
/// - **macOS**: extract a complete signed `.app`, then rename the bundle with rollback.
/// - **Windows**: extract zip to a staging directory, rename-and-replace strategy.
pub async fn install(archive: &Path, version: &str) -> Result<()> {
    semver::Version::parse(version).context("Invalid update version")?;
    #[cfg(target_os = "linux")]
    {
        install_linux(archive, version).await
    }
    #[cfg(target_os = "macos")]
    {
        install_macos(archive).await
    }
    #[cfg(target_os = "windows")]
    {
        install_windows(archive, version).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = archive;
        anyhow::bail!("Auto-update is not supported on this platform");
    }
}

#[cfg(target_os = "linux")]
async fn install_linux(archive: &Path, version: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let current_exe =
        std::env::current_exe().context("Failed to determine current executable path")?;
    let parent = current_exe
        .parent()
        .context("Executable has no parent directory")?;

    // Extract tar.gz to a temporary directory next to the binary
    let staging = parent.join(".shelldeck-update-staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let status = tokio::process::Command::new("tar")
        .args(["xzf", &archive.to_string_lossy(), "-C"])
        .arg(&staging)
        .status()
        .await
        .context("Failed to run tar")?;

    if !status.success() {
        anyhow::bail!("tar extraction failed");
    }

    // Find the new binary inside the extracted directory
    let new_binary = find_binary_in_dir(&staging)?;

    let browser_payload = publish_browser_payload(&new_binary, parent, version)?;
    // Staged before the application binary is touched, published after it:
    // a failure here leaves the installed files as they were.
    let sessiond_payload = SessiondPayload::stage(&new_binary, parent)?;

    // Stage the replacement on the same filesystem, then swap with rollback.
    let backup = current_exe.with_extension("bak");
    let replacement = current_exe.with_extension("new");
    let _ = std::fs::remove_file(&replacement);
    std::fs::copy(&new_binary, &replacement).context("Failed to stage new binary")?;
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o755))?;
    atomic_replace_file(&current_exe, &replacement, &backup)?;
    browser_payload.commit();
    sessiond_payload.commit();

    // Cleanup
    let _ = std::fs::remove_dir_all(&staging);

    tracing::info!("Linux update installed successfully");
    Ok(())
}

#[cfg(target_os = "linux")]
fn atomic_replace_file(current: &Path, replacement: &Path, backup: &Path) -> Result<()> {
    let _ = std::fs::remove_file(backup);
    std::fs::rename(current, backup).context("Failed to create binary backup")?;
    if let Err(error) = std::fs::rename(replacement, current) {
        let _ = std::fs::rename(backup, current);
        return Err(error).context("Failed to atomically install new binary");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn install_macos(archive: &Path) -> Result<()> {
    let current_exe =
        std::env::current_exe().context("Failed to determine current executable path")?;

    // Walk up to find the .app bundle (e.g. ShellDeck.app/Contents/MacOS/shelldeck)
    let app_bundle = current_exe
        .ancestors()
        .find(|p| p.extension().map_or(false, |ext| ext == "app"))
        .context("Could not find .app bundle")?
        .to_path_buf();

    let staging = app_bundle.with_extension("update-staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let status = tokio::process::Command::new("unzip")
        .args(["-o", &archive.to_string_lossy(), "-d"])
        .arg(&staging)
        .status()
        .await
        .context("Failed to unzip update")?;

    if !status.success() {
        anyhow::bail!("unzip extraction failed");
    }

    let new_bundle = find_app_bundle(&staging)?;
    let backup = app_bundle.with_extension("app.update-backup");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(&app_bundle, &backup).context("Failed to back up current app bundle")?;
    if let Err(error) = std::fs::rename(&new_bundle, &app_bundle) {
        let _ = std::fs::rename(&backup, &app_bundle);
        return Err(error).context("Failed to atomically install new app bundle");
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir_all(&staging);

    tracing::info!("macOS update installed successfully");
    Ok(())
}

#[cfg(target_os = "macos")]
fn find_app_bundle(dir: &Path) -> Result<std::path::PathBuf> {
    if dir.extension().is_some_and(|ext| ext == "app") {
        return Ok(dir.to_path_buf());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "app") {
            return Ok(path);
        }
        if path.is_dir() {
            if let Ok(bundle) = find_app_bundle(&path) {
                return Ok(bundle);
            }
        }
    }
    anyhow::bail!("Downloaded macOS archive does not contain an .app bundle")
}

#[cfg(target_os = "windows")]
async fn install_windows(archive: &Path, version: &str) -> Result<()> {
    let current_exe =
        std::env::current_exe().context("Failed to determine current executable path")?;
    let parent = current_exe
        .parent()
        .context("Executable has no parent directory")?;

    let staging = parent.join(".shelldeck-update-staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    // Use PowerShell to extract zip. The paths are interpolated into
    // single-quoted PowerShell strings, where an embedded quote is escaped by
    // doubling it — otherwise a `'` in the install path breaks the command
    // (same escaping as the clipboard capture in issue_attachments.rs).
    let archive_arg = archive.to_string_lossy().replace('\'', "''");
    let staging_arg = staging.to_string_lossy().replace('\'', "''");
    let status = tokio::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Expand-Archive -Force -Path '{archive_arg}' -DestinationPath '{staging_arg}'"
            ),
        ])
        .status()
        .await
        .context("Failed to extract zip")?;

    if !status.success() {
        anyhow::bail!("PowerShell zip extraction failed");
    }

    let new_binary = find_binary_in_dir(&staging)?;

    let browser_payload = publish_browser_payload(&new_binary, parent, version)?;
    let old = current_exe.with_extension("old.exe");
    pending_replace_file(&current_exe, &new_binary, &old)?;
    browser_payload.commit();

    let _ = std::fs::remove_dir_all(&staging);

    tracing::info!("Windows update installed successfully");
    Ok(())
}

/// Swap a running Windows executable, keeping a rollback.
///
/// Windows will not let a running `.exe` be replaced in place, so the current
/// one is renamed aside and the new one copied into its name; the leftover is
/// cleaned up on the next launch.
///
/// The rename must be undone when the copy fails. Without that, a copy
/// interrupted by a full disk, a locked file or an antivirus leaves the user
/// with **no executable at all** — only a `.old.exe` they would have to rename
/// back by hand. The Unix path has always rolled back; this one had not.
#[cfg(target_os = "windows")]
fn pending_replace_file(current: &Path, replacement: &Path, old: &Path) -> Result<()> {
    let _ = std::fs::remove_file(old);
    std::fs::rename(current, old).context("Failed to rename current exe to .old.exe")?;
    if let Err(error) = std::fs::copy(replacement, current) {
        // Put the user's working executable back before surfacing the failure.
        let _ = std::fs::rename(old, current);
        return Err(error).context("Failed to copy new binary");
    }
    Ok(())
}

/// Publish a new version of CEF before replacing the main executable. Existing
/// versions stay at their original paths: Chromium launches child processes from
/// that path and Windows keeps its DLLs locked while the browser is running.
#[cfg(any(target_os = "linux", target_os = "windows"))]
struct BrowserPayloadPublication {
    path: Option<std::path::PathBuf>,
}
#[cfg(any(target_os = "linux", target_os = "windows"))]
impl BrowserPayloadPublication {
    fn commit(mut self) {
        self.path.take();
    }
}
#[cfg(any(target_os = "linux", target_os = "windows"))]
impl Drop for BrowserPayloadPublication {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn publish_browser_payload(
    new_binary: &Path,
    install_dir: &Path,
    expected_version: &str,
) -> Result<BrowserPayloadPublication> {
    let source_root = new_binary
        .parent()
        .context("Update executable has no parent")?
        .join("browser");
    let mut entries =
        std::fs::read_dir(&source_root).context("Update archive is missing its browser engine")?;
    let source = entries
        .next()
        .context("Browser engine directory is empty")??
        .path();
    anyhow::ensure!(
        entries.next().is_none(),
        "Update archive must contain one browser engine version"
    );
    let version = source
        .file_name()
        .and_then(|v| v.to_str())
        .context("Invalid browser version directory")?;
    semver::Version::parse(version).context("Invalid browser engine version")?;
    anyhow::ensure!(
        version == expected_version,
        "Browser engine version does not match the signed release"
    );
    anyhow::ensure!(
        source.symlink_metadata()?.is_dir(),
        "Browser engine must be a real directory"
    );
    let helper = if cfg!(target_os = "windows") {
        "shelldeck-browser.exe"
    } else {
        "shelldeck-browser"
    };
    let cef = if cfg!(target_os = "windows") {
        "libcef.dll"
    } else {
        "libcef.so"
    };
    for required in [
        helper,
        cef,
        "icudtl.dat",
        "resources.pak",
        "chrome_100_percent.pak",
        "chrome_200_percent.pak",
        "v8_context_snapshot.bin",
        "locales/en-US.pak",
        "runtime.json",
    ] {
        anyhow::ensure!(
            source
                .join(required)
                .symlink_metadata()
                .map(|m| m.is_file())
                .unwrap_or(false),
            "Browser engine is missing {required}"
        );
    }
    #[cfg(target_os = "windows")]
    for required in ["shelldeck-browser.dll", "chrome_elf.dll"] {
        anyhow::ensure!(
            source.join(required).is_file(),
            "Browser engine is missing {required}"
        );
    }
    let metadata: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(source.join("runtime.json"))?)
            .context("Invalid browser runtime metadata")?;
    anyhow::ensure!(
        metadata["shelldeck_version"].as_str() == Some(expected_version),
        "Browser runtime metadata does not match the signed release"
    );
    anyhow::ensure!(
        metadata["platform"].as_str()
            == Some(if cfg!(target_os = "windows") {
                "windows"
            } else {
                "linux"
            }),
        "Browser runtime targets a different platform"
    );
    let destination_root = install_dir.join("browser");
    std::fs::create_dir_all(&destination_root)?;
    let destination = destination_root.join(version);
    anyhow::ensure!(
        !destination.exists(),
        "Browser engine version {version} already exists; the update was not applied"
    );
    // Both paths live beside the executable. Existing Chromium processes retain
    // their version paths; only an uncommitted new payload is removed on error.
    std::fs::rename(&source, &destination).context("Failed to publish browser engine")?;
    Ok(BrowserPayloadPublication {
        path: Some(destination),
    })
}

/// Session daemon files a Linux archive may carry beside the application
/// binary: the daemon for this machine and its static builds for SSH
/// servers (`scripts/package-release.sh`). The application looks for all of
/// them in the directory of its own executable.
#[cfg(target_os = "linux")]
const SESSIOND_FILES: [&str; 3] = [
    "shelldeck-sessiond",
    "shelldeck-sessiond-x86_64-unknown-linux-musl",
    "shelldeck-sessiond-aarch64-unknown-linux-musl",
];

/// Larger than any real build, far below anything that would fill a disk.
#[cfg(target_os = "linux")]
const MAX_SESSIOND_BYTES: u64 = 256 * 1024 * 1024;

/// The session daemon files of an update, copied next to the installed
/// ones under a temporary name. [`commit`](Self::commit) renames them into
/// place; dropping without it removes the copies and leaves the installed
/// files untouched.
///
/// A daemon that is running is not disturbed: its file is replaced by
/// rename, so the running process keeps the file it was started from and
/// goes on serving its terminals. The new file is used by the next daemon
/// that starts.
#[cfg(target_os = "linux")]
struct SessiondPayload {
    /// (staged copy, final path)
    staged: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    /// Installed builds for SSH servers that the archive does not
    /// replace. Each is the build of an older release: the application
    /// refuses to upload it (it does not carry the new release's marker),
    /// so it is deleted once the update went through.
    stale: Vec<std::path::PathBuf>,
}

#[cfg(target_os = "linux")]
impl SessiondPayload {
    /// Copy what the archive carries. An archive without any of the files
    /// is accepted (nothing is staged, the installed files stay); a file
    /// that is present must be a real ELF file.
    fn stage(new_binary: &Path, install_dir: &Path) -> Result<Self> {
        use std::io::Read;
        use std::os::unix::fs::PermissionsExt;

        let source_dir = new_binary
            .parent()
            .context("Update executable has no parent")?;
        let mut payload = Self {
            staged: Vec::new(),
            stale: Vec::new(),
        };
        for name in SESSIOND_FILES {
            let source = source_dir.join(name);
            let Ok(metadata) = source.symlink_metadata() else {
                // The local daemon stays when an archive has none (it
                // negotiates with whatever application talks to it). A
                // build for SSH servers is tied to its release.
                let installed = install_dir.join(name);
                if name.ends_with("-unknown-linux-musl")
                    && installed
                        .symlink_metadata()
                        .is_ok_and(|metadata| metadata.is_file())
                {
                    payload.stale.push(installed);
                }
                continue;
            };
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= MAX_SESSIOND_BYTES,
                "Update archive carries an invalid session daemon: {name}"
            );
            let mut magic = [0u8; 4];
            std::fs::File::open(&source)
                .and_then(|mut file| file.read_exact(&mut magic))
                .with_context(|| format!("Failed to read session daemon {name}"))?;
            anyhow::ensure!(
                magic == *b"\x7fELF",
                "Update archive carries an invalid session daemon: {name}"
            );
            let staged = install_dir.join(format!(".{name}.update"));
            let _ = std::fs::remove_file(&staged);
            // Recorded first, so a failed copy is cleaned up too.
            payload
                .staged
                .push((staged.clone(), install_dir.join(name)));
            std::fs::copy(&source, &staged)
                .with_context(|| format!("Failed to stage session daemon {name}"))?;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(payload)
    }

    /// Publish the staged files. The application binary is already
    /// replaced at this point, so a failure is logged, not returned: the
    /// application works without the daemon file (in-process terminals).
    fn commit(mut self) {
        for (staged, destination) in self.staged.drain(..) {
            if let Err(error) = std::fs::rename(&staged, &destination) {
                tracing::warn!("Failed to publish {}: {error}", destination.display());
                let _ = std::fs::remove_file(&staged);
            }
        }
        for stale in self.stale.drain(..) {
            if let Err(error) = std::fs::remove_file(&stale) {
                tracing::warn!("Failed to delete {}: {error}", stale.display());
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for SessiondPayload {
    fn drop(&mut self) {
        for (staged, _) in self.staged.drain(..) {
            let _ = std::fs::remove_file(staged);
        }
    }
}

/// Walk a directory to find the `shelldeck` binary (or `shelldeck.exe` on Windows).
fn find_binary_in_dir(dir: &Path) -> Result<std::path::PathBuf> {
    let target_name = if cfg!(target_os = "windows") {
        "shelldeck.exe"
    } else {
        "shelldeck"
    };

    for entry in walkdir(dir)? {
        if entry.file_name().is_some_and(|n| n == target_name) {
            return Ok(entry);
        }
    }

    anyhow::bail!(
        "Could not find '{}' in extracted archive at {}",
        target_name,
        dir.display()
    );
}

/// Simple recursive directory walk returning file paths.
fn walkdir(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut results = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path)?);
            } else {
                results.push(path);
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn fixture_release(payload: &'static [u8], sha256: String) -> ReleaseInfo {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(payload).await.unwrap();
        });
        ReleaseInfo {
            platform: "linux-x86_64".into(),
            version: "1.0.0".into(),
            url: format!("http://{address}/update"),
            sha256,
            size: payload.len() as u64,
            pub_date: "2026-07-23T12:00:00Z".into(),
            signature: String::new(),
        }
    }

    // SDTEST-2781 / SDUC-283
    #[tokio::test]
    async fn interrupted_download_cleans_partial_and_preserves_existing_destination() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let count = stream.read(&mut request).await.unwrap();
            assert!(std::str::from_utf8(&request[..count])
                .unwrap()
                .starts_with("GET /update "));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\nConnection: close\r\n\r\nshort",
                )
                .await
                .unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("update.tar.gz");
        std::fs::write(&dest, b"previous verified archive").unwrap();
        let release = ReleaseInfo {
            platform: "linux-x86_64".into(),
            version: "1.2.3".into(),
            url: format!("http://{address}/update"),
            sha256: "a".repeat(64),
            size: 32,
            pub_date: "2026-10-30T00:00:00Z".into(),
            signature: String::new(),
        };
        assert!(download_verified_payload(&release, &dest).await.is_err());
        server.await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"previous verified archive");
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            1,
            "failed network transfer left a partial archive"
        );
        let mut empty = release;
        empty.size = 0;
        assert!(download_verified_payload(&empty, &dest)
            .await
            .unwrap_err()
            .to_string()
            .contains("empty payload size"));
    }

    // SDTEST-2588 — SDUC-557
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn browser_engine_update_preserves_live_versions_and_rolls_back_failed_binary_swap() {
        let temp = tempfile::tempdir().unwrap();
        let install = temp.path().join("install");
        let stage = temp.path().join("stage");
        let version = stage.join("browser/1.2.3");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::create_dir_all(install.join("browser/1.0.0")).unwrap();
        std::fs::write(install.join("browser/1.0.0/live-resource"), b"in-use").unwrap();
        let helper = if cfg!(target_os = "windows") {
            "shelldeck-browser.exe"
        } else {
            "shelldeck-browser"
        };
        let cef = if cfg!(target_os = "windows") {
            "libcef.dll"
        } else {
            "libcef.so"
        };
        for name in [
            helper,
            cef,
            "icudtl.dat",
            "shelldeck-browser.dll",
            "chrome_elf.dll",
            "resources.pak",
            "chrome_100_percent.pak",
            "chrome_200_percent.pak",
            "v8_context_snapshot.bin",
            "locales/en-US.pak",
        ] {
            std::fs::create_dir_all(version.join(name).parent().unwrap()).unwrap();
            std::fs::write(version.join(name), b"engine").unwrap();
        }
        std::fs::write(version.join("runtime.json"), serde_json::to_vec(&serde_json::json!({"shelldeck_version":"1.2.3", "platform":if cfg!(target_os = "windows") { "windows" } else { "linux" }})).unwrap()).unwrap();
        let binary = stage.join("shelldeck");
        let publication = publish_browser_payload(&binary, &install, "1.2.3").unwrap();
        assert!(install.join("browser/1.2.3").is_dir());
        assert_eq!(
            std::fs::read(install.join("browser/1.0.0/live-resource")).unwrap(),
            b"in-use"
        );
        drop(publication);
        assert!(!install.join("browser/1.2.3").exists());
        assert!(install.join("browser/1.0.0/live-resource").is_file());
    }

    // SDTEST-2589 — SDUC-557
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn incomplete_browser_payload_fails_before_changing_installed_files() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        let install = temp.path().join("install");
        std::fs::create_dir_all(stage.join("browser/1.2.3")).unwrap();
        let error = publish_browser_payload(&stage.join("shelldeck"), &install, "1.2.3")
            .err()
            .unwrap();
        assert!(error.to_string().contains("Browser engine is missing"));
        assert!(!install.exists());
    }

    // SDTEST-3033 (SDUC-574): the session daemon files of an update are
    // staged without touching the installed ones, published by rename and
    // removed again when the update does not go through.
    #[cfg(target_os = "linux")]
    #[test]
    fn sdtest_3033_session_daemon_files_are_staged_then_published_or_rolled_back() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let install = temp.path().join("install");
        let stage = temp.path().join("stage");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        let binary = stage.join("shelldeck");
        let daemon = "shelldeck-sessiond";
        let remote = "shelldeck-sessiond-aarch64-unknown-linux-musl";
        std::fs::write(install.join(daemon), b"\x7fELF installed daemon").unwrap();
        let listing = |dir: &Path| {
            let mut names: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };

        // An archive from before the daemon shipped: nothing to do, and
        // the installed daemon is kept.
        SessiondPayload::stage(&binary, &install).unwrap().commit();
        assert_eq!(listing(&install), [daemon]);
        assert_eq!(
            std::fs::read(install.join(daemon)).unwrap(),
            b"\x7fELF installed daemon"
        );

        // Staged: the installed file is still the old one.
        std::fs::write(stage.join(daemon), b"\x7fELF new daemon").unwrap();
        std::fs::write(stage.join(remote), b"\x7fELF new remote build").unwrap();
        // Not one of the shipped names: ignored.
        std::fs::write(stage.join("shelldeck-sessiond-extra"), b"\x7fELF").unwrap();
        let payload = SessiondPayload::stage(&binary, &install).unwrap();
        assert_eq!(
            std::fs::read(install.join(daemon)).unwrap(),
            b"\x7fELF installed daemon"
        );
        assert!(!install.join(remote).exists());
        // The update fails after staging (the binary swap, say): the
        // copies are removed and nothing changed.
        drop(payload);
        assert_eq!(listing(&install), [daemon]);
        assert_eq!(
            std::fs::read(install.join(daemon)).unwrap(),
            b"\x7fELF installed daemon"
        );

        // The update goes through: both files are in place, executable,
        // and no temporary name is left.
        SessiondPayload::stage(&binary, &install).unwrap().commit();
        assert_eq!(listing(&install), [daemon, remote]);
        assert_eq!(
            std::fs::read(install.join(daemon)).unwrap(),
            b"\x7fELF new daemon"
        );
        for name in [daemon, remote] {
            let mode = std::fs::metadata(install.join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        // An archive without the build for SSH servers (a release built
        // without it, or a rollback): the installed one belongs to another
        // release and is deleted, but only once the update went through.
        // The local daemon is replaced as usual.
        std::fs::remove_file(stage.join(remote)).unwrap();
        std::fs::write(stage.join(daemon), b"\x7fELF newer daemon").unwrap();
        drop(SessiondPayload::stage(&binary, &install).unwrap());
        assert_eq!(listing(&install), [daemon, remote]);
        SessiondPayload::stage(&binary, &install).unwrap().commit();
        assert_eq!(listing(&install), [daemon]);
        assert_eq!(
            std::fs::read(install.join(daemon)).unwrap(),
            b"\x7fELF newer daemon"
        );
        std::fs::write(stage.join(daemon), b"\x7fELF new daemon").unwrap();
        std::fs::write(stage.join(remote), b"\x7fELF new remote build").unwrap();
        SessiondPayload::stage(&binary, &install).unwrap().commit();
        assert_eq!(listing(&install), [daemon, remote]);

        // A file that is not an ELF executable, or a symlink, stops the
        // update before anything is staged or replaced.
        std::fs::write(stage.join(remote), b"#!/bin/sh\n").unwrap();
        let error = SessiondPayload::stage(&binary, &install).err().unwrap();
        assert!(error.to_string().contains("invalid session daemon"));
        std::fs::remove_file(stage.join(remote)).unwrap();
        std::fs::remove_file(stage.join(daemon)).unwrap();
        std::os::unix::fs::symlink("/bin/sh", stage.join(daemon)).unwrap();
        let error = SessiondPayload::stage(&binary, &install).err().unwrap();
        assert!(error.to_string().contains("invalid session daemon"));
        assert_eq!(listing(&install), [daemon, remote]);
        assert_eq!(
            std::fs::read(install.join(daemon)).unwrap(),
            b"\x7fELF new daemon"
        );
    }

    // SDTEST-1240 — a bad digest never publishes the destination archive.
    #[tokio::test]
    async fn sha256_mismatch_removes_partial_download() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("update.tar.gz");
        let release = fixture_release(b"not the signed archive", "0".repeat(64)).await;

        let error = download_verified_payload(&release, &destination)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("SHA-256 mismatch"));
        assert!(!destination.exists());
        assert!(!destination.with_extension("part").exists());
    }

    // SDTEST-1243 — the Windows replacement keeps a rollback, and restores it
    // when the copy fails.
    //
    // Windows cannot overwrite a running exe, so the current one is renamed
    // aside first. If the copy then fails and nothing undoes that rename, the
    // user is left with no executable at all — a failed update uninstalls the
    // app. The Unix path always rolled back; this one did not.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_pending_replace_keeps_a_rollback_and_restores_it_on_failure() {
        let temp = tempfile::tempdir().unwrap();
        let current = temp.path().join("shelldeck.exe");
        let replacement = temp.path().join("staged.exe");
        let old = temp.path().join("shelldeck.old.exe");
        std::fs::write(&current, b"old").unwrap();
        std::fs::write(&replacement, b"new").unwrap();

        pending_replace_file(&current, &replacement, &old).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert_eq!(
            std::fs::read(&old).unwrap(),
            b"old",
            "the previous executable must survive as a rollback",
        );

        // Now fail the copy by pointing at a replacement that does not exist.
        std::fs::write(&current, b"live").unwrap();
        let missing = temp.path().join("not-there.exe");
        let error = pending_replace_file(&current, &missing, &old).unwrap_err();
        assert!(error.to_string().contains("Failed to copy new binary"));
        assert_eq!(
            std::fs::read(&current).unwrap(),
            b"live",
            "a failed update must leave the user's executable in place",
        );
    }

    // SDTEST-1244 — an archive without the expected binary fails before
    // anything is replaced.
    #[test]
    fn an_archive_without_the_binary_fails_before_touching_the_installation() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        std::fs::create_dir_all(staging.join("nested")).unwrap();
        std::fs::write(staging.join("README.md"), b"not a binary").unwrap();
        std::fs::write(staging.join("nested").join("shelldeck.txt"), b"decoy").unwrap();

        let error = find_binary_in_dir(&staging).unwrap_err();
        assert!(
            error.to_string().contains("Could not find"),
            "unexpected error: {error}",
        );

        // The guard runs before any swap, so nothing outside the staging
        // directory has been touched.
        assert!(staging.join("README.md").exists());
    }

    // SDTEST-1242 — the Unix replacement is a same-filesystem rename and
    // retains the previous executable as a rollback copy.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_binary_replacement_is_atomic_and_keeps_backup() {
        let temp = tempfile::tempdir().unwrap();
        let current = temp.path().join("shelldeck");
        let replacement = temp.path().join("shelldeck.new");
        let backup = temp.path().join("shelldeck.bak");
        std::fs::write(&current, b"old").unwrap();
        std::fs::write(&replacement, b"new").unwrap();

        atomic_replace_file(&current, &replacement, &backup).unwrap();

        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
        assert!(!replacement.exists());
    }
}
