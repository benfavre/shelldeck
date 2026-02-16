use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncWriteExt;

use crate::ReleaseInfo;

/// Download the release archive to `dest`, verifying its SHA-256 digest.
pub async fn download_and_verify(release: &ReleaseInfo, dest: &Path) -> Result<()> {
    tracing::info!("Downloading update from {}", release.url);

    let response = reqwest::get(&release.url)
        .await
        .context("Failed to download update")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Update download failed with HTTP {}",
            response.status().as_u16()
        );
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read update payload")?;

    // Verify SHA-256
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = format!("{:x}", hasher.finalize());

    if digest != release.sha256 {
        anyhow::bail!(
            "SHA-256 mismatch: expected {}, got {}",
            release.sha256,
            digest
        );
    }

    tracing::info!("SHA-256 verified: {}", digest);

    // Write to destination
    let mut file = tokio::fs::File::create(dest)
        .await
        .context("Failed to create update file")?;
    file.write_all(&bytes)
        .await
        .context("Failed to write update file")?;
    file.flush().await?;

    Ok(())
}

/// Platform-specific installation of the downloaded archive.
///
/// - **Linux**: extract tar.gz, rename running binary to `.bak`, copy new binary in place.
/// - **macOS**: extract zip, rsync into the running app bundle.
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

    // Rename current binary to .bak, copy new one in place
    let backup = current_exe.with_extension("bak");
    std::fs::rename(&current_exe, &backup).context("Failed to rename current binary to .bak")?;
    std::fs::copy(&new_binary, &current_exe).context("Failed to copy new binary")?;
    std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755))?;

    // Cleanup
    let _ = std::fs::remove_dir_all(&staging);

    tracing::info!("Linux update installed successfully");
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

    // rsync the extracted .app contents into the current bundle
    let status = tokio::process::Command::new("rsync")
        .args(["-a", "--delete"])
        .arg(format!("{}/", staging.to_string_lossy()))
        .arg(format!("{}/", app_bundle.to_string_lossy()))
        .status()
        .await
        .context("Failed to rsync update")?;

    if !status.success() {
        anyhow::bail!("rsync failed");
    }

    let _ = std::fs::remove_dir_all(&staging);

    tracing::info!("macOS update installed successfully");
    Ok(())
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
