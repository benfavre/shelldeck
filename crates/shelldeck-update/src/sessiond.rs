//! The session daemon files as a small download of their own
//! (docs/design/session-daemon.md, section 15, "Daemon files that are
//! missing").
//!
//! The Linux tar.gz carries `shelldeck-sessiond` and its static builds for
//! SSH servers beside the application binary, and the updater installs
//! them. An install that was updated by a release from before the daemon
//! (0.20.0 and older replace the application binary only) has the new
//! application and none of these files. Rather than wait for the next
//! update, the application fetches them: a release publishes the same
//! files as a second, small archive under its own key of the signed update
//! manifest.
//!
//! The same rules as an application update apply: the manifest entry is
//! signed by the offline update key and names this platform, the archive
//! matches the signed size and SHA-256, and only the known file names are
//! taken out of it. On top of that the entry must be the release of this
//! very application: a build for SSH servers is tied to its release.

use std::path::Path;

/// The daemon for this machine, as named in the archive and on disk.
pub const LOCAL_DAEMON: &str = "shelldeck-sessiond";

/// Far above the real archive (a few megabytes).
#[cfg(target_os = "linux")]
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// Key of the daemon files in the update manifest, when a release
/// publishes them for this platform.
pub fn payload_platform() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64-sessiond")
    } else {
        None
    }
}

/// Whether this build can fetch the daemon files at all: a platform that
/// has them and the update verification key (release builds only).
pub fn offered() -> bool {
    payload_platform().is_some() && crate::embedded_verification_key().is_some()
}

/// Why the daemon files were not fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchErrorKind {
    /// This build cannot fetch them ([`offered`] is false).
    NotOffered,
    /// The latest release does not publish them.
    NotPublished,
    /// The latest release is another version than this application. The
    /// files come with the update to it.
    OtherVersion(String),
    /// The update server or the archive could not be reached, or the
    /// archive is not the signed one.
    Download,
    /// The manifest entry or the content of the archive was refused.
    Invalid,
    /// The files could not be written.
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub kind: FetchErrorKind,
    /// For the log, never shown as is.
    pub detail: String,
}

impl FetchError {
    fn new(kind: FetchErrorKind, detail: impl std::fmt::Display) -> Self {
        Self {
            kind,
            detail: detail.to_string(),
        }
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}

/// Fetch the daemon files of this release into `dest` (created when
/// missing). Blocks on the network: call it from a background thread,
/// never from inside a Tokio runtime.
///
/// Files already in `dest` are replaced by rename, so a daemon started
/// from one of them keeps running. On an error nothing in `dest` changed.
pub fn fetch_blocking(dest: &Path) -> Result<(), FetchError> {
    #[cfg(target_os = "linux")]
    {
        let (Some(platform), Some(key)) = (payload_platform(), crate::embedded_verification_key())
        else {
            return Err(FetchError::new(
                FetchErrorKind::NotOffered,
                "no daemon download for this build",
            ));
        };
        crate::http_runtime().block_on(linux::fetch(platform, key, dest))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dest;
        Err(FetchError::new(
            FetchErrorKind::NotOffered,
            "no daemon download for this platform",
        ))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{FetchError, FetchErrorKind, LOCAL_DAEMON, MAX_ARCHIVE_BYTES};
    use crate::installer::{self, SessiondPayload};
    use crate::ReleaseInfo;
    use std::path::Path;
    use std::time::Duration;

    pub(super) async fn fetch(platform: &str, key: &str, dest: &Path) -> Result<(), FetchError> {
        let download =
            |error: &dyn std::fmt::Display| FetchError::new(FetchErrorKind::Download, error);
        let url = format!(
            "{}/api/releases/latest?platform={platform}",
            crate::UPDATE_SERVER
        );
        let response = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| download(&error))?
            .get(&url)
            .send()
            .await
            .map_err(|error| download(&error))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FetchError::new(
                FetchErrorKind::NotPublished,
                "the latest release has no session daemon archive",
            ));
        }
        if !response.status().is_success() {
            return Err(download(&format!(
                "the update server answered HTTP {}",
                response.status().as_u16()
            )));
        }
        let release: ReleaseInfo = response
            .json()
            .await
            .map_err(|error| FetchError::new(FetchErrorKind::Invalid, error))?;
        check_manifest(&release, key, platform, crate::CURRENT_VERSION)?;

        let write = |error: std::io::Error| FetchError::new(FetchErrorKind::Write, error);
        std::fs::create_dir_all(dest).map_err(write)?;
        // Inside `dest`: the files are published by rename, which needs
        // the same filesystem. Removed when this function returns.
        let work = tempfile::Builder::new()
            .prefix(".fetch-")
            .tempdir_in(dest)
            .map_err(write)?;
        let archive = work.path().join("sessiond.tar.gz");
        installer::download_verified_payload(&release, &archive)
            .await
            .map_err(|error| download(&format!("{error:#}")))?;
        install_archive(&archive, work.path(), dest).await
    }

    /// The manifest entry is signed for `platform`, is the release
    /// `version` and describes a plausible archive.
    pub(super) fn check_manifest(
        release: &ReleaseInfo,
        key: &str,
        platform: &str,
        version: &str,
    ) -> Result<(), FetchError> {
        let invalid = |detail: &str| FetchError::new(FetchErrorKind::Invalid, detail);
        crate::verify_release_for_platform(release, key, platform)
            .map_err(|error| FetchError::new(FetchErrorKind::Invalid, error))?;
        if release.version != version {
            return Err(FetchError::new(
                FetchErrorKind::OtherVersion(release.version.clone()),
                format!("the archive is for {}, this is {version}", release.version),
            ));
        }
        if !release.url.starts_with("https://") {
            return Err(invalid("the archive address is not https"));
        }
        if release.size == 0 || release.size > MAX_ARCHIVE_BYTES {
            return Err(invalid("the archive size is not plausible"));
        }
        Ok(())
    }

    /// Unpack `archive` under `work` and publish the daemon files it
    /// carries into `dest`. `work` is on the filesystem of `dest`.
    pub(super) async fn install_archive(
        archive: &Path,
        work: &Path,
        dest: &Path,
    ) -> Result<(), FetchError> {
        let invalid =
            |detail: &dyn std::fmt::Display| FetchError::new(FetchErrorKind::Invalid, detail);
        let stage = work.join("stage");
        std::fs::create_dir_all(&stage)
            .map_err(|error| FetchError::new(FetchErrorKind::Write, error))?;
        let status = tokio::process::Command::new("tar")
            .arg("xzf")
            .arg(archive)
            .args(["--no-same-owner", "--no-same-permissions", "-C"])
            .arg(&stage)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|error| invalid(&format!("tar could not run: {error}")))?;
        if !status.success() {
            return Err(invalid(&"the archive could not be unpacked"));
        }
        // Takes the known names only and checks each one (a regular ELF
        // file of a plausible size). Anything else in the archive stays in
        // `work` and is deleted with it.
        let payload = SessiondPayload::stage_from(&stage, dest)
            .map_err(|error| invalid(&format!("{error:#}")))?;
        if !payload.carries(LOCAL_DAEMON) {
            return Err(invalid(&"the archive has no shelldeck-sessiond"));
        }
        payload.commit();
        if !dest.join(LOCAL_DAEMON).is_file() {
            return Err(FetchError::new(
                FetchErrorKind::Write,
                "the daemon could not be put in place",
            ));
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::linux::{check_manifest, install_archive};
    use super::{FetchErrorKind, LOCAL_DAEMON};
    use crate::{release_signing_message, ReleaseInfo};
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use std::path::Path;

    const PLATFORM: &str = "linux-x86_64-sessiond";

    fn signed(platform: &str, version: &str, url: &str, size: u64) -> (ReleaseInfo, String) {
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().as_bytes());
        let mut release = ReleaseInfo {
            platform: platform.into(),
            version: version.into(),
            url: url.into(),
            sha256: "b".repeat(64),
            size,
            pub_date: "2026-11-11T12:00:00Z".into(),
            signature: String::new(),
        };
        release.signature = base64::engine::general_purpose::STANDARD.encode(
            signing_key
                .sign(release_signing_message(&release).as_bytes())
                .to_bytes(),
        );
        (release, public_key)
    }

    // SDTEST-3057 (SDUC-574): the manifest entry of the daemon files is
    // accepted only when it is signed for the daemon key of this platform
    // and is the release of this application.
    #[test]
    fn sdtest_3057_daemon_manifest_entry_must_be_signed_for_this_release() {
        let url = "https://example.test/shelldeck-sessiond-linux-x86_64.tar.gz";
        let (release, key) = signed(PLATFORM, "1.2.3", url, 4096);
        check_manifest(&release, &key, PLATFORM, "1.2.3").unwrap();

        // The entry of the application archive, validly signed, is not
        // one for the daemon files.
        let (app, key) = signed("linux-x86_64", "1.2.3", url, 4096);
        let error = check_manifest(&app, &key, PLATFORM, "1.2.3").unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);

        // Another release: its builds for SSH servers do not belong here.
        let (newer, key) = signed(PLATFORM, "1.2.4", url, 4096);
        let error = check_manifest(&newer, &key, PLATFORM, "1.2.3").unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::OtherVersion("1.2.4".into()));

        // Changed after signing.
        let (mut tampered, key) = signed(PLATFORM, "1.2.3", url, 4096);
        tampered.sha256 = "c".repeat(64);
        let error = check_manifest(&tampered, &key, PLATFORM, "1.2.3").unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);

        // Signed, but not something to download.
        let (plain, key) = signed(PLATFORM, "1.2.3", "http://example.test/a.tar.gz", 4096);
        let error = check_manifest(&plain, &key, PLATFORM, "1.2.3").unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);
        let (huge, key) = signed(PLATFORM, "1.2.3", url, u64::MAX);
        let error = check_manifest(&huge, &key, PLATFORM, "1.2.3").unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);
    }

    fn archive_of(dir: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let content = dir.join("content");
        std::fs::create_dir_all(&content).unwrap();
        for (name, bytes) in files {
            std::fs::write(content.join(name), bytes).unwrap();
        }
        let archive = dir.join("sessiond.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("czf")
            .arg(&archive)
            .arg("-C")
            .arg(&content)
            .args(files.iter().map(|(name, _)| *name))
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(&content).unwrap();
        archive
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // SDTEST-3058 (SDUC-574): the downloaded archive puts the daemon files
    // in place, executable, and nothing else; an archive that is not one
    // leaves the directory as it was.
    #[tokio::test]
    async fn sdtest_3058_daemon_archive_is_installed_or_refused_whole() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("sessiond");
        std::fs::create_dir_all(&dest).unwrap();
        let remote = "shelldeck-sessiond-x86_64-unknown-linux-musl";

        // No daemon for this machine in it: refused, nothing written.
        let work = temp.path().join("work-a");
        std::fs::create_dir_all(&work).unwrap();
        let archive = archive_of(&work, &[(remote, b"\x7fELF remote")]);
        let error = install_archive(&archive, &work, &dest).await.unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);
        assert!(listing(&dest).is_empty());

        // A file under the daemon's name that is not a program: refused.
        let work = temp.path().join("work-b");
        std::fs::create_dir_all(&work).unwrap();
        let archive = archive_of(&work, &[(LOCAL_DAEMON, b"#!/bin/sh\n")]);
        let error = install_archive(&archive, &work, &dest).await.unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);
        assert!(listing(&dest).is_empty());

        // Not an archive.
        let work = temp.path().join("work-c");
        std::fs::create_dir_all(&work).unwrap();
        let archive = work.join("sessiond.tar.gz");
        std::fs::write(&archive, b"not gzip").unwrap();
        let error = install_archive(&archive, &work, &dest).await.unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::Invalid);
        assert!(listing(&dest).is_empty());

        // The real shape: both files land, executable; the extra file of
        // the archive does not.
        let work = temp.path().join("work-d");
        std::fs::create_dir_all(&work).unwrap();
        let archive = archive_of(
            &work,
            &[
                (LOCAL_DAEMON, b"\x7fELF daemon"),
                (remote, b"\x7fELF remote"),
                ("README", b"not a daemon file"),
            ],
        );
        install_archive(&archive, &work, &dest).await.unwrap();
        assert_eq!(listing(&dest), [LOCAL_DAEMON, remote]);
        for name in [LOCAL_DAEMON, remote] {
            let mode = std::fs::metadata(dest.join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        assert_eq!(
            std::fs::read(dest.join(LOCAL_DAEMON)).unwrap(),
            b"\x7fELF daemon"
        );
    }
}
