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

    let mut response = reqwest::get(&release.url)
        .await
        .context("Failed to download update")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Update download failed with HTTP {}",
            response.status().as_u16()
        );
    }

    let partial = dest.with_extension("part");
    let _ = tokio::fs::remove_file(&partial).await;
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
        if release.size > 0 && downloaded > release.size {
            let _ = tokio::fs::remove_file(&partial).await;
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

    if release.size > 0 && downloaded != release.size {
        let _ = tokio::fs::remove_file(&partial).await;
        anyhow::bail!(
            "Update size mismatch: expected {}, got {}",
            release.size,
            downloaded
        );
    }

    let digest = format!("{:x}", hasher.finalize());

    if !digest.eq_ignore_ascii_case(&release.sha256) {
        let _ = tokio::fs::remove_file(&partial).await;
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
pub async fn install(archive: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        install_linux(archive).await
    }
    #[cfg(target_os = "macos")]
    {
        install_macos(archive).await
    }
    #[cfg(target_os = "windows")]
    {
        install_windows(archive).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = archive;
        anyhow::bail!("Auto-update is not supported on this platform");
    }
}

#[cfg(target_os = "linux")]
async fn install_linux(archive: &Path) -> Result<()> {
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

    // Stage the replacement on the same filesystem, then swap with rollback.
    let backup = current_exe.with_extension("bak");
    let replacement = current_exe.with_extension("new");
    let _ = std::fs::remove_file(&replacement);
    std::fs::copy(&new_binary, &replacement).context("Failed to stage new binary")?;
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o755))?;
    atomic_replace_file(&current_exe, &replacement, &backup)?;

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
async fn install_windows(archive: &Path) -> Result<()> {
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

    // Use PowerShell to extract zip
    let status = tokio::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Expand-Archive -Force -Path '{}' -DestinationPath '{}'",
                archive.to_string_lossy(),
                staging.to_string_lossy()
            ),
        ])
        .status()
        .await
        .context("Failed to extract zip")?;

    if !status.success() {
        anyhow::bail!("PowerShell zip extraction failed");
    }

    let new_binary = find_binary_in_dir(&staging)?;

    // On Windows we cannot replace a running exe directly. Rename current to .old,
    // copy new one in, and the old one will be cleaned up on next launch.
    let old = current_exe.with_extension("old.exe");
    let _ = std::fs::remove_file(&old); // Remove previous .old if it exists
    std::fs::rename(&current_exe, &old).context("Failed to rename current exe to .old.exe")?;
    std::fs::copy(&new_binary, &current_exe).context("Failed to copy new binary")?;

    let _ = std::fs::remove_dir_all(&staging);

    tracing::info!("Windows update installed successfully");
    Ok(())
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
