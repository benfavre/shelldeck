//! Opt-in session daemon backend for local terminals
//! (`general.session_daemon`, docs/design/session-daemon.md section 10).
//!
//! One process-wide `Supervisor` for the per-user `shelldeck-sessiond`
//! daemon. A pane opened here is a `TerminalSession` like any other for
//! the view: dropping it detaches (the pane keeps running, so it survives
//! an app quit or update), `TerminalSession::terminate` ends it (tab or
//! pane closed by the user).
//!
//! Nothing here waits for the daemon on the calling (GPUI) thread:
//! [`spawn`] and [`attach`] return a session in its "connecting" phase and
//! a thread per session connects, starts the daemon when needed, attaches,
//! and reattaches after a connection loss. [`restorable_panes`] is the one
//! blocking call and must run on a background thread.
//!
//! Limits, listed in docs/testing/session-daemon-qa.md: Unix only,
//! unscoped terminals only (workspace terminals keep their in-process PTY
//! and its directory authority).

use shelldeck_terminal::session::TerminalSession;
use uuid::Uuid;

/// Label of the panes this backend spawns. Only those are offered back at
/// startup, so panes made by other tools (the debug CLI) stay out.
#[cfg(unix)]
const PANE_LABEL: &str = "shelldeck-local";

#[cfg(unix)]
mod imp {
    use super::PANE_LABEL;
    use crate::t;
    use shelldeck_sessiond::attached::SessionOptions;
    use shelldeck_sessiond::client::SpawnOptions;
    use shelldeck_sessiond::paths;
    use shelldeck_sessiond::supervisor::{
        DaemonTarget, Notices, PaneTarget, Supervisor, SupervisorOptions,
    };
    use shelldeck_terminal::session::TerminalSession;
    use std::path::PathBuf;
    use std::sync::{Arc, OnceLock};
    use uuid::Uuid;

    static SUPERVISOR: OnceLock<Result<Supervisor, String>> = OnceLock::new();

    /// Environment passed to a new pane. The daemon scrubs its own copy of
    /// these, so each pane gets the display and agent of the app that
    /// spawned it. Names outside the protocol allowlist are never sent.
    const FORWARDED_ENV: &[&str] = &[
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "LC_MESSAGES",
        "COLORTERM",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "SSH_AUTH_SOCK",
    ];
    /// `MAX_ENV_VALUE` of the protocol.
    const MAX_ENV_VALUE: usize = 1024;

    /// The process-wide supervisor. Creating it starts one idle worker
    /// thread and connects to nothing. The notices are translated once,
    /// with the language in use at that moment.
    fn supervisor() -> Result<&'static Supervisor, String> {
        SUPERVISOR
            .get_or_init(|| {
                let notice = |key: &str| Some(format!("[{}]", t!(key)));
                Supervisor::new(
                    // Runs on the connecting thread, never on the UI one:
                    // finding the run directory may probe a socket.
                    Arc::new(|| {
                        Ok(DaemonTarget {
                            dir: paths::default_run_dir().map_err(|error| error.to_string())?,
                            explicit_dir: false,
                            daemon_exe: daemon_binary(),
                        })
                    }),
                    SupervisorOptions {
                        notices: Notices {
                            connecting: notice("terminal.session_daemon.connecting"),
                            connection_lost: notice("terminal.session_daemon.connection_lost"),
                            ended: notice("terminal.session_daemon.ended"),
                            unavailable: notice("terminal.session_daemon.unavailable"),
                        },
                        ..SupervisorOptions::default()
                    },
                )
                .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The daemon binary: `SHELLDECK_SESSIOND_BIN` (development), else
    /// `shelldeck-sessiond` next to the app executable. Always an absolute
    /// path to an existing file, never a `PATH` lookup.
    fn daemon_binary() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("SHELLDECK_SESSIOND_BIN").map(PathBuf::from) {
            if path.is_absolute() && path.is_file() {
                return Ok(path);
            }
            return Err("SHELLDECK_SESSIOND_BIN is not an absolute path to a file".into());
        }
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let path = exe
            .parent()
            .ok_or("the app executable has no directory")?
            .join("shelldeck-sessiond");
        if path.is_file() {
            Ok(path)
        } else {
            Err(format!("{} is not installed", path.display()))
        }
    }

    fn pane_env() -> Vec<(String, String)> {
        let mut env = vec![("TERM".to_string(), "xterm-256color".to_string())];
        for name in FORWARDED_ENV {
            if let Ok(value) = std::env::var(name) {
                if !value.is_empty() && value.len() <= MAX_ENV_VALUE {
                    env.push(((*name).to_string(), value));
                }
            }
        }
        env
    }

    pub(super) fn spawn(
        shell: Option<&str>,
        rows: u16,
        cols: u16,
        scrollback: usize,
    ) -> Result<TerminalSession, String> {
        // One `stat`, so a build without the daemon binary falls back to
        // an in-process terminal at once instead of showing a dead tab.
        daemon_binary()?;
        supervisor()?
            .open(
                PaneTarget::Spawn(SpawnOptions {
                    label: PANE_LABEL.into(),
                    // Empty: the daemon starts the login shell in the home
                    // directory.
                    argv: shell
                        .map(|shell| vec![shell.to_string()])
                        .unwrap_or_default(),
                    env: pane_env(),
                    ..SpawnOptions::default()
                }),
                SessionOptions {
                    title: "Terminal".into(),
                    rows,
                    cols,
                    scrollback,
                    keep_pane_size: false,
                },
            )
            .map_err(|error| error.to_string())
    }

    pub(super) fn attach(
        pane: Uuid,
        title: &str,
        rows: u16,
        cols: u16,
        scrollback: usize,
    ) -> Result<TerminalSession, String> {
        supervisor()?
            .open(
                PaneTarget::Existing(pane),
                SessionOptions {
                    title: title.into(),
                    rows,
                    cols,
                    scrollback,
                    // The view has not been laid out yet at startup, and a
                    // background tab is not laid out until it is shown: the
                    // pane keeps its size (and its history its width) until
                    // this session sends a real one.
                    keep_pane_size: true,
                },
            )
            .map_err(|error| error.to_string())
    }

    pub(super) fn restorable_panes() -> Vec<Uuid> {
        let supervisor = match supervisor() {
            Ok(supervisor) => supervisor,
            Err(error) => {
                tracing::warn!("session daemon panes not restored: {error}");
                return Vec::new();
            }
        };
        let panes = match supervisor.list_panes(false) {
            Ok(Some(panes)) => panes,
            Ok(None) => return Vec::new(),
            Err(error) => {
                tracing::warn!("session daemon panes not listed: {error}");
                return Vec::new();
            }
        };
        let open = supervisor.open_panes();
        panes
            .into_iter()
            .filter(|info| info.alive && info.label == PANE_LABEL && !open.contains(&info.pane))
            .map(|info| info.pane)
            .collect()
    }
}

/// Open a session on a new pane of the local session daemon. Returns at
/// once: the daemon is reached (and started when absent) on a thread of
/// the session, which shows a notice until the pane's screen arrives.
/// `shell`: the configured shell, else the login shell. An error means
/// "use an in-process terminal instead" (no daemon binary, not Unix).
pub(super) fn spawn(
    shell: Option<&str>,
    rows: u16,
    cols: u16,
    scrollback: usize,
) -> Result<TerminalSession, String> {
    #[cfg(unix)]
    {
        imp::spawn(shell, rows, cols, scrollback)
    }
    #[cfg(not(unix))]
    {
        let _ = (shell, rows, cols, scrollback);
        Err("the session daemon is not available on this platform".into())
    }
}

/// Open a session on a pane that already runs (layout restore). Returns at
/// once, like [`spawn`]. The pane keeps its size until the view asks for
/// one; `rows` and `cols` only size the local grid until the snapshot
/// arrives. A pane that is gone by then is reported in the session.
pub(super) fn attach(
    pane: Uuid,
    title: &str,
    rows: u16,
    cols: u16,
    scrollback: usize,
) -> Result<TerminalSession, String> {
    #[cfg(unix)]
    {
        imp::attach(pane, title, rows, cols, scrollback)
    }
    #[cfg(not(unix))]
    {
        let _ = (pane, title, rows, cols, scrollback);
        Err("the session daemon is not available on this platform".into())
    }
}

/// Live panes this backend spawned earlier and that no session shows yet
/// (app restart), in the daemon's order. Never starts a daemon.
///
/// Blocking (a connect and a round trip on the daemon socket): call it
/// from a background thread, never from the GPUI thread.
pub(crate) fn restorable_panes() -> Vec<Uuid> {
    #[cfg(unix)]
    {
        imp::restorable_panes()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}
