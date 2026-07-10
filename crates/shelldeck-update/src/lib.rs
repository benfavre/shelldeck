pub mod installer;
pub mod platform;

use gpui::*;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// The running application version, set at compile time from workspace Cargo.toml.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Base URL of the update server (Cloudflare Worker).
pub const UPDATE_SERVER: &str = "https://shelldeck.1clic.pro";

/// How often to check for updates (default: 60 minutes).
const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Delay before the first update check after launch.
const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(10);

/// Shared Tokio runtime for HTTP requests. GPUI's background executor
/// does not provide a Tokio reactor, so reqwest/hyper calls must run here.
fn http_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("shelldeck-http")
            .build()
            .expect("Failed to create HTTP tokio runtime")
    })
}

/// Status of the auto-updater, suitable for display in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoUpdateStatus {
    Idle,
    Checking,
    Downloading,
    Installing,
    UpdateAvailable(String),
    Updated(String),
    Errored(String),
}

impl std::fmt::Display for AutoUpdateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "ShellDeck v{}", CURRENT_VERSION),
            Self::Checking => write!(f, "Checking for updates..."),
            Self::Downloading => write!(f, "Downloading update..."),
            Self::Installing => write!(f, "Installing update..."),
            Self::UpdateAvailable(v) => write!(f, "Update {} available", v),
            Self::Updated(v) => write!(f, "Updated to {} — restart to apply", v),
            Self::Errored(msg) => write!(f, "Update error: {}", msg),
        }
    }
}

/// Release information returned by the update server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub pub_date: String,
}

/// Events emitted by the auto-updater for the workspace to react to.
#[derive(Debug, Clone)]
pub enum AutoUpdateEvent {
    StatusChanged(AutoUpdateStatus),
}

/// GPUI entity that manages background update polling.
pub struct AutoUpdater {
    pub status: AutoUpdateStatus,
    enabled: bool,
    check_interval: Duration,
    latest_release: Option<ReleaseInfo>,
    _poll_task: Option<Task<()>>,
}

impl EventEmitter<AutoUpdateEvent> for AutoUpdater {}

impl Default for AutoUpdater {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoUpdater {
    pub fn new() -> Self {
        Self {
            status: AutoUpdateStatus::Idle,
            enabled: false,
            check_interval: DEFAULT_CHECK_INTERVAL,
            latest_release: None,
            _poll_task: None,
        }
    }

    /// Begin the polling loop. Should be called once after entity creation.
    pub fn start_polling(&mut self, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }

        let interval = self.check_interval;

        self._poll_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            // Wait before the first check
            cx.background_executor().timer(INITIAL_CHECK_DELAY).await;

            loop {
                let enabled = this.read_with(cx, |u, _| u.enabled).unwrap_or(false);
                if !enabled {
                    break;
                }
                let _ = this.update(cx, |u, cx| {
                    u.check_for_update(cx);
                });

                cx.background_executor().timer(interval).await;
            }
        }));
    }

    /// Perform a single update check right now.
    pub fn check_for_update(&mut self, cx: &mut Context<Self>) {
        self.set_status(AutoUpdateStatus::Checking, cx);

        let platform = platform::current_platform();
        let url = format!(
            "{}/api/releases/latest?platform={}",
            UPDATE_SERVER, platform
        );

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = http_runtime()
                .spawn(async move {
                    let response = reqwest::get(&url).await?;
                    if !response.status().is_success() {
                        anyhow::bail!("Server returned HTTP {}", response.status().as_u16());
                    }
                    let release: ReleaseInfo = response.json().await?;
                    Ok(release)
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))
                .and_then(|r| r);

            match result {
                Ok(release) => {
                    let _ = this.update(cx, |u, cx| {
                        u.handle_release_check(release, cx);
                    });
                }
                Err(e) => {
                    tracing::warn!("Update check failed: {}", e);
                    let _ = this.update(cx, |u, cx| {
                        u.set_status(AutoUpdateStatus::Errored(e.to_string()), cx);
                    });
                }
            }
        })
        .detach();
    }

    fn handle_release_check(&mut self, release: ReleaseInfo, cx: &mut Context<Self>) {
        let current = match semver::Version::parse(CURRENT_VERSION) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to parse current version: {}", e);
                self.set_status(AutoUpdateStatus::Idle, cx);
                return;
            }
        };

        let remote = match semver::Version::parse(&release.version) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "Failed to parse remote version '{}': {}",
                    release.version,
                    e
                );
                self.set_status(AutoUpdateStatus::Idle, cx);
                return;
            }
        };

        if remote > current {
            tracing::info!(
                "Update available: {} -> {}",
                CURRENT_VERSION,
                release.version
            );
            self.latest_release = Some(release.clone());
            self.set_status(
                AutoUpdateStatus::UpdateAvailable(release.version.clone()),
                cx,
            );
            if self.enabled {
                self.download_and_install(release, cx);
            }
        } else {
            tracing::info!("Already up to date ({})", CURRENT_VERSION);
            self.set_status(AutoUpdateStatus::Idle, cx);
        }
    }

    /// Download, verify, and install the given release.
    pub fn download_and_install(&mut self, release: ReleaseInfo, cx: &mut Context<Self>) {
        self.set_status(AutoUpdateStatus::Downloading, cx);

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // Download phase runs on the Tokio runtime (needs reactor for HTTP)
            let download_result = http_runtime()
                .spawn(async move {
                    let tmp_dir = tempfile::tempdir()?;
                    let archive_name = if cfg!(target_os = "linux") {
                        "update.tar.gz"
                    } else {
                        "update.zip"
                    };
                    let archive_path = tmp_dir.path().join(archive_name);

                    installer::download_and_verify(&release, &archive_path).await?;
                    Ok::<(tempfile::TempDir, std::path::PathBuf, String), anyhow::Error>((
                        tmp_dir,
                        archive_path,
                        release.version.clone(),
                    ))
                })
                .await
                .map_err(|e| anyhow::anyhow!("Task join error: {}", e))
                .and_then(|r| r);

            match download_result {
                Ok((_tmp_dir, archive_path, version)) => {
                    let _ = this.update(cx, |u, cx| {
                        u.set_status(AutoUpdateStatus::Installing, cx);
                    });

                    // Install phase also runs on the Tokio runtime (uses tokio::process)
                    let install_result = http_runtime()
                        .spawn(async move {
                            installer::install(&archive_path).await?;
                            Ok::<String, anyhow::Error>(version)
                        })
                        .await
                        .map_err(|e| anyhow::anyhow!("Task join error: {}", e))
                        .and_then(|r| r);

                    match install_result {
                        Ok(version) => {
                            let _ = this.update(cx, |u, cx| {
                                u.set_status(AutoUpdateStatus::Updated(version), cx);
                            });
                        }
                        Err(e) => {
                            tracing::error!("Update installation failed: {}", e);
                            let _ = this.update(cx, |u, cx| {
                                u.set_status(AutoUpdateStatus::Errored(e.to_string()), cx);
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Update download failed: {}", e);
                    let _ = this.update(cx, |u, cx| {
                        u.set_status(AutoUpdateStatus::Errored(e.to_string()), cx);
                    });
                }
            }
        })
        .detach();
    }

    /// Manually trigger the download/install of a previously detected update.
    pub fn trigger_update(&mut self, cx: &mut Context<Self>) {
        if let Some(release) = self.latest_release.take() {
            self.download_and_install(release, cx);
        }
    }

    /// Enable or disable auto-update polling.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.enabled = enabled;
        if enabled {
            if self._poll_task.is_none() {
                self.start_polling(cx);
            }
        } else {
            self._poll_task = None;
        }
    }

    fn set_status(&mut self, status: AutoUpdateStatus, cx: &mut Context<Self>) {
        self.status = status.clone();
        cx.emit(AutoUpdateEvent::StatusChanged(status));
        cx.notify();
    }
}

#[cfg(test)]
mod release_parity_tests {
    // SDTEST-1260/1261 — release manifest ↔ install scripts key parity.
    //
    // The three moving parts of a release (workflow, worker, client)
    // must agree on the same `{os}-{arch}` string, or a fresh install
    // silently 404s on the affected OS. Rather than mock a real
    // Wrangler + KV setup, we read the source files at compile time
    // via `include_str!` and assert the platform keys appear as
    // literal strings in every shipping location.
    //
    // This is the highest-value cheap test in my inventory: a rename
    // in one place without the others is the exact regression class
    // that keeps us from noticing until an end user reports "install
    // script doesn't work on Mac".

    const WORKFLOW: &str = include_str!("../../../.github/workflows/release.yml");
    const WORKER: &str = include_str!("../../../cloudflare/update-worker/src/index.ts");

    /// Every platform key `{os}-{arch}` that the release pipeline builds
    /// AND the update worker serves. Must be a literal string in each
    /// source file. `macos-x86_64` is served by the worker but not
    /// currently built by the workflow matrix (documented drift); we
    /// only enforce parity on the keys the workflow actually publishes.
    const SHIPPING_KEYS: &[&str] = &["linux-x86_64", "macos-aarch64", "windows-x86_64"];

    // SDTEST-1260 — every shipping platform key appears verbatim in the
    // release workflow AND the update worker. If either drops one, the
    // release job or the fresh install fails.
    #[test]
    fn every_shipping_key_appears_in_release_workflow() {
        for key in SHIPPING_KEYS {
            assert!(
                WORKFLOW.contains(key),
                "release.yml missing platform key {key:?} — release matrix drift",
            );
        }
    }

    #[test]
    fn every_shipping_key_appears_in_update_worker() {
        for key in SHIPPING_KEYS {
            assert!(
                WORKER.contains(key),
                "worker/src/index.ts missing platform key {key:?} — install / update endpoint would 404",
            );
        }
    }

    // SDTEST-1261 — `darwin-*` MUST NEVER appear as a platform key.
    // AGENTS.md § release: "Platform keys are {os}-{arch} and use
    // **`macos-*`**, never **`darwin-*`**". The rustc identifier
    // is `darwin` but the release contract renames it to `macos`;
    // a drift here is silent and only surfaces at install time.
    #[test]
    fn darwin_prefix_is_forbidden_in_release_contract() {
        for forbidden in ["darwin-x86_64", "darwin-aarch64", "darwin-arm64"] {
            assert!(
                !WORKFLOW.contains(forbidden),
                "release.yml uses forbidden `{forbidden}` — AGENTS.md mandates `macos-*`",
            );
            assert!(
                !WORKER.contains(forbidden),
                "worker/src/index.ts uses forbidden `{forbidden}` — must be `macos-*`",
            );
        }
    }

    // Runtime `current_platform()` must produce a string that matches
    // one of the release keys (or a known-unsupported build target).
    // Guards the round-trip: build ↔ platform-string ↔ manifest key.
    #[test]
    fn current_platform_matches_a_release_key_or_is_explicitly_unsupported() {
        let p = crate::platform::current_platform();
        let known = SHIPPING_KEYS.iter().any(|k| p == *k);
        let unsupported = p.contains("unknown");
        assert!(
            known || unsupported,
            "current_platform()={p:?} matches neither a shipping key {SHIPPING_KEYS:?} nor an `unknown` fallback",
        );
    }
}
