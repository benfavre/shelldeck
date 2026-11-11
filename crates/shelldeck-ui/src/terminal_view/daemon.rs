//! Opt-in session daemon backend for local terminals
//! (`general.session_daemon`, docs/design/session-daemon.md section 10).
//!
//! One process-wide connection to the per-user `shelldeck-sessiond` daemon.
//! A pane spawned here is a `TerminalSession` like any other for the view:
//! dropping it detaches (the pane keeps running, so it survives an app
//! quit or update), `TerminalSession::terminate` ends it (tab or pane
//! closed by the user).
//!
//! Prototype limits, listed in docs/testing/session-daemon-qa.md: Unix
//! only, unscoped terminals only (workspace terminals keep their in-process
//! PTY and its directory authority), blocking calls on the UI thread, no
//! reconnect when the daemon goes away.

use shelldeck_terminal::session::TerminalSession;

/// Label of the panes this backend spawns. Only those are offered back at
/// startup, so panes made by other tools (the debug CLI) stay out.
#[cfg(unix)]
const PANE_LABEL: &str = "shelldeck-local";

#[cfg(unix)]
mod imp {
    use super::PANE_LABEL;
    use crate::t;
    use parking_lot::Mutex;
    use shelldeck_sessiond::attached::{LocalSessions, SessionOptions, SessionsOptions};
    use shelldeck_sessiond::client::SpawnOptions;
    use shelldeck_sessiond::paths;
    use shelldeck_terminal::session::TerminalSession;
    use std::path::PathBuf;

    static CONNECTION: Mutex<Option<LocalSessions>> = Mutex::new(None);

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

    fn options() -> SessionsOptions {
        SessionsOptions {
            connection_lost_notice: Some(t!("terminal.session_daemon.connection_lost").to_string()),
            ..SessionsOptions::default()
        }
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

    /// The shared connection. `start`: launch the daemon when none runs.
    fn connection(start: bool) -> Result<Option<LocalSessions>, String> {
        let mut slot = CONNECTION.lock();
        if let Some(sessions) = slot.as_ref().filter(|sessions| !sessions.is_closed()) {
            return Ok(Some(sessions.clone()));
        }
        *slot = None;
        let dir = paths::default_run_dir().map_err(|error| error.to_string())?;
        let sessions = if start {
            let binary = daemon_binary()?;
            Some(
                LocalSessions::connect(&dir, false, &binary, options())
                    .map_err(|error| error.to_string())?,
            )
        } else {
            LocalSessions::connect_existing(&dir, options()).map_err(|error| error.to_string())?
        };
        slot.clone_from(&sessions);
        Ok(sessions)
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
        let sessions = connection(true)?.ok_or("the session daemon is not running")?;
        let spawned = sessions
            .spawn(
                SpawnOptions {
                    label: PANE_LABEL.into(),
                    // Empty: the daemon starts the login shell in the home
                    // directory.
                    argv: shell
                        .map(|shell| vec![shell.to_string()])
                        .unwrap_or_default(),
                    env: pane_env(),
                    ..SpawnOptions::default()
                },
                SessionOptions {
                    title: "Terminal".into(),
                    rows,
                    cols,
                    scrollback,
                    keep_pane_size: false,
                },
            )
            .map_err(|error| error.to_string())?;
        tracing::info!(pane = %spawned.pane, pid = spawned.pid, "spawned a session daemon pane");
        Ok(spawned.session)
    }

    pub(super) fn restore(rows: u16, cols: u16, scrollback: usize) -> Vec<TerminalSession> {
        let sessions = match connection(false) {
            Ok(Some(sessions)) => sessions,
            Ok(None) => return Vec::new(),
            Err(error) => {
                tracing::warn!("session daemon panes not restored: {error}");
                return Vec::new();
            }
        };
        let panes = match sessions.list_panes() {
            Ok(panes) => panes,
            Err(error) => {
                tracing::warn!("session daemon panes not listed: {error}");
                return Vec::new();
            }
        };
        let attached = sessions.attached_panes();
        let mut restored = Vec::new();
        for info in panes {
            if !info.alive || info.label != PANE_LABEL || attached.contains(&info.pane) {
                continue;
            }
            let options = SessionOptions {
                title: "Terminal".into(),
                rows,
                cols,
                scrollback,
                // The view has not been laid out yet at startup, and a
                // background tab is not laid out until it is shown: the
                // pane keeps its size (and its history its width) until
                // this session sends a real one.
                keep_pane_size: true,
            };
            match sessions.attach(info.pane, options) {
                Ok(session) => restored.push(session),
                Err(error) => tracing::warn!(pane = %info.pane, "pane not restored: {error}"),
            }
        }
        restored
    }
}

/// Spawn a pane on the local session daemon (started when absent) and
/// attach a session to it. `shell`: the configured shell, else the login
/// shell. An error means "use an in-process terminal instead".
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

/// Attach a session to every live pane this backend spawned earlier and
/// that is not shown yet (app restart). Never starts a daemon. `rows` and
/// `cols` only size the local grid until the snapshot arrives: the panes
/// keep their own size until the view asks for one.
pub(super) fn restore(rows: u16, cols: u16, scrollback: usize) -> Vec<TerminalSession> {
    #[cfg(unix)]
    {
        imp::restore(rows, cols, scrollback)
    }
    #[cfg(not(unix))]
    {
        let _ = (rows, cols, scrollback);
        Vec::new()
    }
}
