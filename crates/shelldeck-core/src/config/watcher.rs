use crate::error::{Result, ShellDeckError};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Watches ~/.ssh/config for changes and invokes a callback.
pub struct ConfigWatcher {
    ssh_config_path: PathBuf,
    _watcher: Option<RecommendedWatcher>,
}

impl ConfigWatcher {
    /// Create a new ConfigWatcher targeting the default ~/.ssh/config.
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        let ssh_config_path = PathBuf::from(home).join(".ssh").join("config");

        Self {
            ssh_config_path,
            _watcher: None,
        }
    }

    /// Create a ConfigWatcher for a specific path (useful for testing).
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            ssh_config_path: path,
            _watcher: None,
        }
    }

    /// Start watching for file changes. Calls `callback` when the config file is modified.
    /// Events are debounced at 500ms to avoid rapid-fire triggers.
    ///
    /// This spawns a background thread for debouncing. The watcher is kept alive
    /// as long as the returned `ConfigWatcher` is not dropped.
    pub fn start<F>(&mut self, callback: F) -> Result<()>
    where
        F: Fn() + Send + 'static,
    {
        let watch_path = if self.ssh_config_path.exists() {
            self.ssh_config_path.clone()
        } else {
            // Watch the .ssh directory instead, in case config doesn't exist yet
            self.ssh_config_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| self.ssh_config_path.clone())
        };

        let target_path = self.ssh_config_path.clone();
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |event: std::result::Result<notify::Event, notify::Error>| {
                if let Ok(event) = event {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            // Check if any of the affected paths match our target
                            let relevant = event.paths.iter().any(|p| p == &target_path);
                            if relevant {
                                let _ = tx.send(());
                            }
                        }
                        _ => {}
                    }
                }
            },
            Config::default(),
        )
        .map_err(|e| ShellDeckError::Config(format!("Failed to create file watcher: {}", e)))?;

        watcher
            .watch(&watch_path, RecursiveMode::NonRecursive)
            .map_err(|e| {
                ShellDeckError::Config(format!(
                    "Failed to watch {}: {}",
                    watch_path.display(),
                    e
                ))
            })?;

        info!("Watching {} for changes", watch_path.display());

        // Debounce thread
        std::thread::Builder::new()
            .name("ssh-config-watcher".to_string())
            .spawn(move || {
                let debounce_duration = Duration::from_millis(500);
                let mut last_trigger = Instant::now() - debounce_duration;

                loop {
                    match rx.recv() {
                        Ok(()) => {
                            let now = Instant::now();
                            if now.duration_since(last_trigger) >= debounce_duration {
                                last_trigger = now;
                                debug!("SSH config changed, triggering callback");
                                callback();
                            } else {
                                debug!("SSH config change debounced");
                            }
                        }
                        Err(_) => {
                            // Channel closed, watcher was dropped
                            debug!("Config watcher channel closed, stopping");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| {
                ShellDeckError::Config(format!("Failed to spawn watcher thread: {}", e))
            })?;

        self._watcher = Some(watcher);
        Ok(())
    }
}
