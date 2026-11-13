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

/// What the local session daemon has to offer back at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
// Without a daemon (non Unix) only `Live` is ever built.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) enum RestorablePanes {
    /// Live panes this backend spawned and no session shows yet, in the
    /// daemon's order. Empty when no daemon runs.
    Live(Vec<Uuid>),
    /// A daemon runs, left by another version of the app, and cannot be
    /// spoken to. Which of the saved panes are alive is unknown; none of
    /// them is touched.
    OtherVersion,
}

/// Label of the panes this backend spawns. Only those are offered back at
/// startup, so panes made by other tools (the debug CLI) stay out.
#[cfg(unix)]
const PANE_LABEL: &str = "shelldeck-local";

#[cfg(unix)]
mod imp {
    use super::{RestorablePanes, PANE_LABEL};
    use crate::t;
    use shelldeck_sessiond::attached::SessionOptions;
    use shelldeck_sessiond::client::SpawnOptions;
    use shelldeck_sessiond::paths;
    use shelldeck_sessiond::protocol::PaneInfo;
    use shelldeck_sessiond::requests::{self, Request};
    use shelldeck_sessiond::supervisor::{
        DaemonTarget, Notices, PaneTarget, Supervisor, SupervisorOptions,
    };
    use shelldeck_terminal::session::TerminalSession;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
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
                            other_version: notice("terminal.session_daemon.other_version"),
                        },
                        ..SupervisorOptions::default()
                    },
                )
                .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The daemon binary (`crate::sessiond_files`): beside the app
    /// executable, copied out of an AppImage or a macOS bundle, or fetched
    /// to the per-user directory. Always an absolute path to an existing
    /// file, never a `PATH` lookup.
    fn daemon_binary() -> Result<PathBuf, String> {
        crate::sessiond_files::local_daemon()
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
        let supervisor = supervisor()?;
        if supervisor.daemon_is_other_version() {
            // The daemon that runs was left by another version and is not
            // replaced while it lives. This terminal is an ordinary one;
            // look again, off this thread, so that daemon panes come back
            // once that daemon is gone.
            static CHECKING: AtomicBool = AtomicBool::new(false);
            if !CHECKING.swap(true, Ordering::SeqCst) {
                let supervisor = supervisor.clone();
                let started = std::thread::Builder::new()
                    .name("sessiond-recheck".into())
                    .spawn(move || {
                        supervisor.recheck();
                        CHECKING.store(false, Ordering::SeqCst);
                    });
                if started.is_err() {
                    CHECKING.store(false, Ordering::SeqCst);
                }
            }
            return Err("the session daemon belongs to another version of ShellDeck".into());
        }
        supervisor
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

    /// A live pane of this backend that nothing shows: not this process
    /// (`open`) and no other client of the daemon (a second instance of
    /// the app). A pane someone is attached to is theirs: taking it would
    /// mirror it in two windows, and closing the tab here would end it
    /// there.
    pub(super) fn restorable(info: &PaneInfo, open: &[Uuid]) -> bool {
        info.alive && info.label == PANE_LABEL && info.clients == 0 && !open.contains(&info.pane)
    }

    pub(super) fn restorable_panes() -> RestorablePanes {
        use shelldeck_sessiond::attached::AttachError;
        let supervisor = match supervisor() {
            Ok(supervisor) => supervisor,
            Err(error) => {
                tracing::warn!("session daemon panes not restored: {error}");
                return RestorablePanes::Live(Vec::new());
            }
        };
        let panes = match supervisor.list_panes(false) {
            Ok(Some(panes)) => panes,
            Ok(None) => return RestorablePanes::Live(Vec::new()),
            Err(AttachError::OtherVersion) => {
                tracing::warn!(
                    "the session daemon belongs to another version of ShellDeck: \
                     it is left running and its panes are not attached"
                );
                return RestorablePanes::OtherVersion;
            }
            Err(error) => {
                tracing::warn!("session daemon panes not listed: {error}");
                return RestorablePanes::Live(Vec::new());
            }
        };
        let open = supervisor.open_panes();
        RestorablePanes::Live(
            panes
                .into_iter()
                .filter(|info| restorable(info, &open))
                .map(|info| info.pane)
                .collect(),
        )
    }

    /// Requests of `shelldeck-sessiond split` (see
    /// `shelldeck_sessiond::requests`): read from the run directory, and
    /// answered there, by one thread, so the view only exchanges them
    /// through memory.
    #[derive(Default)]
    struct SplitRequests {
        started: AtomicBool,
        waiting: parking_lot::Mutex<Vec<(Uuid, Request)>>,
        answers: parking_lot::Mutex<Vec<(Uuid, Result<(), String>)>>,
    }

    static SPLIT_REQUESTS: OnceLock<SplitRequests> = OnceLock::new();

    fn split_requests() -> &'static SplitRequests {
        let state = SPLIT_REQUESTS.get_or_init(SplitRequests::default);
        if !state.started.swap(true, Ordering::SeqCst) {
            let started = std::thread::Builder::new()
                .name("sessiond-requests".into())
                .spawn(move || {
                    let mut dir: Option<PathBuf> = None;
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(150));
                        // Finding the run directory may probe a socket:
                        // done here, once it succeeds.
                        if dir.is_none() {
                            dir = paths::default_run_dir().ok();
                        }
                        let Some(dir) = dir.as_deref() else { continue };
                        for (id, result) in std::mem::take(&mut *state.answers.lock()) {
                            requests::answer(
                                dir,
                                id,
                                result.as_ref().map(|_| ()).map_err(String::as_str),
                            );
                        }
                        let taken = requests::take(dir);
                        if !taken.is_empty() {
                            state.waiting.lock().extend(taken);
                        }
                    }
                });
            if started.is_err() {
                state.started.store(false, Ordering::SeqCst);
            }
        }
        state
    }

    pub(super) fn take_requests() -> Vec<(Uuid, Request)> {
        std::mem::take(&mut *split_requests().waiting.lock())
    }

    pub(super) fn answer_request(id: Uuid, result: Result<(), String>) {
        split_requests().answers.lock().push((id, result));
    }

    #[cfg(test)]
    mod tests {
        use super::{restorable, PANE_LABEL};
        use shelldeck_sessiond::protocol::PaneInfo;
        use uuid::Uuid;

        // SDTEST-3042 (SDUC-574): at startup only live panes of this
        // backend that nobody shows are offered back. A pane another
        // client is attached to (a second instance of the app) is left
        // to it.
        #[test]
        fn sdtest_3042_panes_shown_by_another_client_are_not_restored() {
            let pane = |label: &str, alive: bool, clients: u16| PaneInfo {
                pane: Uuid::new_v4(),
                group: Uuid::nil(),
                label: label.into(),
                title: String::new(),
                cwd: "/".into(),
                rows: 24,
                cols: 80,
                created_at: 0,
                alive,
                exit_code: 0,
                clients,
                offset: 0,
            };
            let free = pane(PANE_LABEL, true, 0);
            assert!(restorable(&free, &[]));
            assert!(!restorable(&free, &[free.pane]), "shown by this app");
            assert!(!restorable(&pane(PANE_LABEL, true, 1), &[]), "attached");
            assert!(!restorable(&pane(PANE_LABEL, true, 3), &[]), "attached");
            assert!(!restorable(&pane(PANE_LABEL, false, 0), &[]), "dead");
            assert!(!restorable(&pane("debug-cli", true, 0), &[]), "foreign");
        }
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
/// (app restart), in the daemon's order, or the fact that the daemon was
/// left by another version. Never starts a daemon.
///
/// Blocking (a connect and a round trip on the daemon socket): call it
/// from a background thread, never from the GPUI thread.
pub(crate) fn restorable_panes() -> RestorablePanes {
    #[cfg(unix)]
    {
        imp::restorable_panes()
    }
    #[cfg(not(unix))]
    {
        RestorablePanes::Live(Vec::new())
    }
}

/// A pane to show beside another one, asked by `shelldeck-sessiond split`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SplitAsk {
    /// The daemon pane to show.
    pub(super) pane: Uuid,
    /// The daemon pane it goes beside.
    pub(super) beside: Uuid,
    /// Below it, else at its right.
    pub(super) down: bool,
    /// Share of the space the new pane takes.
    pub(super) ratio: f32,
}

/// What `shelldeck-sessiond focus|rename|zoom|unzoom|show` asks for a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PaneAction {
    Focus,
    Rename(String),
    Zoom,
    Unzoom,
    /// Open the pane as a tab, under this name when it is not empty.
    Show(String),
}

/// Something a command asked of the application.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PaneAsk {
    Split(SplitAsk),
    Act { pane: Uuid, action: PaneAction },
}

/// The requests that arrived since the last call, each with the id to
/// answer it. Never blocks: a thread reads them from the run directory.
/// The first call starts it.
pub(super) fn take_requests() -> Vec<(Uuid, PaneAsk)> {
    #[cfg(unix)]
    {
        use shelldeck_sessiond::requests::{Action, Request, Side};
        imp::take_requests()
            .into_iter()
            .map(|(id, request)| {
                let ask = match request {
                    Request::Split(split) => PaneAsk::Split(SplitAsk {
                        pane: split.pane,
                        beside: split.beside,
                        down: split.side == Side::Down,
                        ratio: split.ratio,
                    }),
                    Request::Act(act) => PaneAsk::Act {
                        pane: act.pane,
                        action: match act.action {
                            Action::Focus => PaneAction::Focus,
                            Action::Rename => PaneAction::Rename(act.text),
                            Action::Zoom => PaneAction::Zoom,
                            Action::Unzoom => PaneAction::Unzoom,
                            Action::Show => PaneAction::Show(act.text),
                        },
                    },
                };
                (id, ask)
            })
            .collect()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// Tell the command that asked whether it was done. Never blocks.
pub(super) fn answer_request(id: Uuid, result: Result<(), String>) {
    #[cfg(unix)]
    {
        imp::answer_request(id, result);
    }
    #[cfg(not(unix))]
    {
        let _ = (id, result);
    }
}
