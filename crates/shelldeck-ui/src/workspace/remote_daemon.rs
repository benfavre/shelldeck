//! Session daemon on SSH hosts (`general.session_daemon`,
//! docs/design/session-daemon.md section 15, "Remote app integration").
//!
//! One [`RemoteHost`] per SSH connection entry, kept for the life of the
//! process: it owns the one SSH connection that carries every daemon pane
//! of that host. The SSH connections live on one small runtime shared by
//! all hosts. Tabs and splits are sessions of the host's supervisor; they
//! are opened without waiting, like local daemon panes.
//!
//! This module also keeps the small bookkeeping the workspace needs
//! around a connect: how many tabs of a host are still connecting (so
//! panes left by an earlier run are offered back once, not once per tab),
//! and which hosts already got their fallback notice or their install
//! question in this run.
//!
//! Unix only, like the local backend: the supervisor is.

use crate::t;
use parking_lot::Mutex;
use shelldeck_core::config::app_config::{AppConfig, GeneralConfig, RemoteDaemonInstall};
use shelldeck_core::models::connection::Connection;
use shelldeck_sessiond::attached::SessionOptions;
use shelldeck_sessiond::client::SpawnOptions;
use shelldeck_sessiond::protocol::PaneInfo;
use shelldeck_sessiond::supervisor::{Notices, PaneTarget, SupervisorOptions, UNLIMITED_ATTEMPTS};
use shelldeck_ssh::client::SshClient;
use shelldeck_ssh::sessiond_host::{
    Dial, HostEvent, InstallFailure, InstallFailureKind, RemoteHost, RemoteHostOptions,
};
use shelldeck_ssh::sessiond_install::{FallbackReason, InstallConsent};
use shelldeck_terminal::session::TerminalSession;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Label of the panes the app spawns on a remote daemon. Only those are
/// offered back, so panes made by other tools on the host stay out.
pub(super) const PANE_LABEL: &str = "shelldeck-remote";

pub(super) type HostEvents = mpsc::UnboundedReceiver<(Uuid, HostEvent)>;

struct Entry {
    host: RemoteHost,
    /// Where the entry dials. A connection edited to point elsewhere gets
    /// a new host.
    target: String,
}

struct State {
    hosts: HashMap<Uuid, Entry>,
    /// Hosts replaced after their connection entry changed. Kept so the
    /// sessions they still show keep working.
    retired: Vec<RemoteHost>,
    /// Tabs of a host that are still connecting.
    pending: HashMap<Uuid, usize>,
    noticed: HashSet<Uuid>,
    asked: HashSet<Uuid>,
    events_tx: mpsc::UnboundedSender<(Uuid, HostEvent)>,
    events_rx: Option<HostEvents>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        Mutex::new(State {
            hosts: HashMap::new(),
            retired: Vec::new(),
            pending: HashMap::new(),
            noticed: HashSet::new(),
            asked: HashSet::new(),
            events_tx,
            events_rx: Some(events_rx),
        })
    })
}

/// The runtime the shared SSH connections live on. Never dropped.
pub(super) fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("sessiond-ssh")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Where the bundled daemon builds are looked up, in order:
/// `SHELLDECK_SESSIOND_ARTIFACT_DIR` (development), the directory of the
/// app binary, then `sessiond/` in the app data directory.
fn artifact_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("SHELLDECK_SESSIOND_ARTIFACT_DIR").map(PathBuf::from) {
        if dir.is_absolute() {
            dirs.push(dir);
        }
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        dirs.push(dir);
    }
    dirs.push(AppConfig::config_dir().join("sessiond"));
    dirs
}

fn dial_target(connection: &Connection) -> String {
    connection.dial_identity()
}

/// The host object of `connection`, created on first use. Nothing is
/// dialed here.
pub(super) fn host_for(connection: &Connection) -> Result<RemoteHost, String> {
    let target = dial_target(connection);
    let mut state = state().lock();
    if let Some(entry) = state.hosts.get(&connection.id) {
        if entry.target == target {
            return Ok(entry.host.clone());
        }
    }
    let handle = runtime()?.handle().clone();
    let dialed = connection.clone();
    let dial: Dial = Arc::new(move || {
        let connection = dialed.clone();
        Box::pin(async move { SshClient::new().connect(&connection).await })
    });
    let notice = |key: &str| Some(format!("[{}]", t!(key)));
    let host = RemoteHost::new(
        handle,
        dial,
        RemoteHostOptions {
            artifact_dirs: artifact_dirs(),
            supervisor: SupervisorOptions {
                notices: Notices {
                    connecting: notice("terminal.session_daemon.connecting"),
                    connection_lost: notice("terminal.session_daemon.connection_lost"),
                    ended: notice("terminal.session_daemon.ended"),
                    unavailable: notice("terminal.session_daemon.unavailable"),
                    other_version: notice("terminal.session_daemon.other_version"),
                },
                // A host may be away for any length of time: its tabs
                // keep trying while they are open.
                max_attempts: UNLIMITED_ATTEMPTS,
                ..SupervisorOptions::default()
            },
            ..RemoteHostOptions::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let (events, connection_id) = (state.events_tx.clone(), connection.id);
    host.set_event_handler(move |event| {
        let _ = events.send((connection_id, event));
    });
    let entry = Entry {
        host: host.clone(),
        target,
    };
    if let Some(replaced) = state.hosts.insert(connection.id, entry) {
        state.retired.push(replaced.host);
    }
    Ok(host)
}

/// The host object of a connection, if one was made in this run.
pub(super) fn existing_host(connection_id: Uuid) -> Option<RemoteHost> {
    state()
        .lock()
        .hosts
        .get(&connection_id)
        .map(|entry| entry.host.clone())
}

/// The host object of a connection, if one was made in this run and it
/// dials `target`.
pub(super) fn existing_host_dialing(connection_id: Uuid, target: &str) -> Option<RemoteHost> {
    state()
        .lock()
        .hosts
        .get(&connection_id)
        .filter(|entry| entry.target == target)
        .map(|entry| entry.host.clone())
}

/// The stream of host events, for the one loop that applies them. `None`
/// once it was taken.
pub(super) fn take_events() -> Option<HostEvents> {
    state().lock().events_rx.take()
}

/// A tab of this host started connecting.
pub(super) fn begin_connect(connection_id: Uuid) {
    *state().lock().pending.entry(connection_id).or_default() += 1;
}

/// A tab of this host stopped connecting (ready, fallback or failure).
/// Returns how many are still connecting.
pub(super) fn end_connect(connection_id: Uuid) -> usize {
    let mut state = state().lock();
    let left = state
        .pending
        .get(&connection_id)
        .map_or(0, |count| count.saturating_sub(1));
    if left == 0 {
        state.pending.remove(&connection_id);
    } else {
        state.pending.insert(connection_id, left);
    }
    left
}

/// `true` the first time in this run: show the fallback notice.
pub(super) fn first_notice(connection_id: Uuid) -> bool {
    state().lock().noticed.insert(connection_id)
}

/// `true` the first time in this run: ask about the install.
pub(super) fn first_ask(connection_id: Uuid) -> bool {
    state().lock().asked.insert(connection_id)
}

/// The recorded answer for a host, as the install rules read it. An
/// answer given while the entry dialed another server or account does not
/// count: the user is asked again.
pub(super) fn consent(general: &GeneralConfig, connection: &Connection) -> InstallConsent {
    match general.remote_daemon_install(connection.id, &dial_target(connection)) {
        Some(RemoteDaemonInstall::Allow) => InstallConsent::Allow,
        Some(RemoteDaemonInstall::Never) => InstallConsent::Never,
        None => InstallConsent::Ask,
    }
}

/// What a tab that just reached the daemon shows, and which other panes
/// come back as tabs of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadyPlan {
    /// The pane the connecting tab attaches to; `None`: spawn a new one.
    pub tab: Option<Uuid>,
    /// Panes offered back as further tabs.
    pub extra_tabs: Vec<Uuid>,
}

/// `panes`: what the daemon listed. `open`: panes already shown by this
/// process. A pane another client is attached to is never offered.
/// `others_connecting`: other tabs of the same host are still connecting
/// (a restored session with several tabs); each of them takes one pane
/// when its turn comes, so the rest is only opened by the last one.
pub(super) fn ready_plan(panes: &[PaneInfo], open: &[Uuid], others_connecting: bool) -> ReadyPlan {
    let mut offered = panes
        .iter()
        .filter(|info| {
            info.alive
                && info.label == PANE_LABEL
                && !open.contains(&info.pane)
                // Shown right now by another client (the app on another
                // machine, same account on the host): it is theirs.
                // Taking it would put two users in one shell, and closing
                // the tab here would end it for them.
                && info.clients == 0
        })
        .map(|info| info.pane);
    let tab = offered.next();
    let extra_tabs = if others_connecting {
        Vec::new()
    } else {
        offered.collect()
    };
    ReadyPlan { tab, extra_tabs }
}

/// A session on a new pane of the host's daemon. Returns at once.
pub(super) fn open_new(
    host: &RemoteHost,
    title: &str,
    rows: u16,
    cols: u16,
    scrollback: usize,
) -> Result<TerminalSession, String> {
    host.supervisor()
        .open(
            PaneTarget::Spawn(SpawnOptions {
                label: PANE_LABEL.into(),
                // Empty argv and cwd: the account's login shell in its
                // home directory.
                env: vec![("TERM".to_string(), "xterm-256color".to_string())],
                ..SpawnOptions::default()
            }),
            SessionOptions {
                title: title.into(),
                rows,
                cols,
                scrollback,
                keep_pane_size: false,
            },
        )
        .map_err(|error| error.to_string())
}

/// A session on a pane an earlier run left on the host. Returns at once.
/// The pane keeps its size until the view sends one.
pub(super) fn open_existing(
    host: &RemoteHost,
    pane: Uuid,
    title: &str,
    rows: u16,
    cols: u16,
    scrollback: usize,
) -> Result<TerminalSession, String> {
    host.supervisor()
        .open(
            PaneTarget::Existing(pane),
            SessionOptions {
                title: title.into(),
                rows,
                cols,
                scrollback,
                keep_pane_size: true,
            },
        )
        .map_err(|error| error.to_string())
}

/// Why a host does not get persistent sessions, for the notice.
/// i18n key of the cause of a failed install.
fn install_failure_key(kind: InstallFailureKind) -> &'static str {
    match kind {
        InstallFailureKind::NoArtifact => "toast.ssh.session_daemon.install_error.no_artifact",
        InstallFailureKind::Checksum => "toast.ssh.session_daemon.install_error.checksum",
        InstallFailureKind::NoChecksumTool => {
            "toast.ssh.session_daemon.install_error.no_checksum_tool"
        }
        InstallFailureKind::CannotExecute => {
            "toast.ssh.session_daemon.install_error.cannot_execute"
        }
        InstallFailureKind::Timeout => "toast.ssh.session_daemon.install_error.timeout",
        InstallFailureKind::Connection => "toast.ssh.session_daemon.install_error.connection",
        InstallFailureKind::Fallback(reason) => fallback_reason_key(reason),
        InstallFailureKind::Other => "toast.ssh.session_daemon.install_error.other",
    }
}

/// The cause of a failed install, in the user's language. The English
/// detail of the failure is for the log.
pub(super) fn install_failure_text(failure: &InstallFailure) -> String {
    t!(install_failure_key(failure.kind)).to_string()
}

pub(super) fn fallback_reason_text(reason: FallbackReason) -> String {
    t!(fallback_reason_key(reason)).to_string()
}

fn fallback_reason_key(reason: FallbackReason) -> &'static str {
    match reason {
        FallbackReason::ProbeFailed => "toast.ssh.session_daemon.reason.probe_failed",
        FallbackReason::UnsupportedPlatform => "toast.ssh.session_daemon.reason.unsupported",
        FallbackReason::NoArtifact => "toast.ssh.session_daemon.reason.no_artifact",
        FallbackReason::ConsentPending => "toast.ssh.session_daemon.reason.not_installed",
        FallbackReason::ConsentDeclined => "toast.ssh.session_daemon.reason.declined",
        FallbackReason::InstallFailed => "toast.ssh.session_daemon.reason.install_failed",
        FallbackReason::Unavailable => "toast.ssh.session_daemon.reason.unavailable",
        FallbackReason::VersionMismatch => "toast.ssh.session_daemon.reason.version_mismatch",
        FallbackReason::ConnectFailed => "toast.ssh.session_daemon.reason.connect_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::{consent, ready_plan, ReadyPlan, PANE_LABEL};
    use shelldeck_core::config::app_config::{AppConfig, RemoteDaemonInstall};
    use shelldeck_core::models::connection::Connection;
    use shelldeck_sessiond::protocol::PaneInfo;
    use shelldeck_ssh::sessiond_install::InstallConsent;
    use uuid::Uuid;

    // SDTEST-3043 (SDUC-574): an "install" answer recorded for a
    // connection entry does not follow the entry to another server. After
    // the host, the user, the port or the jump host is edited (the entry
    // keeps its id), the answer reads as "ask": nothing is uploaded to the
    // new server without a new answer.
    #[test]
    fn sdtest_3043_an_edited_connection_is_asked_again() {
        let mut general = AppConfig::default().general;
        let mut entry =
            Connection::new_manual("prod".into(), "prod-a.example".into(), "deploy".into());
        assert_eq!(consent(&general, &entry), InstallConsent::Ask);
        general.set_remote_daemon_install(
            entry.id,
            &entry.dial_identity(),
            RemoteDaemonInstall::Allow,
        );
        assert_eq!(consent(&general, &entry), InstallConsent::Allow);
        // Display-only edits keep the answer.
        entry.alias = "production".into();
        entry.group = Some("web".into());
        assert_eq!(consent(&general, &entry), InstallConsent::Allow);

        let edits: [fn(&mut Connection); 4] = [
            |entry| entry.hostname = "prod-b.example".into(),
            |entry| entry.user = "root".into(),
            |entry| entry.port = 2222,
            |entry| entry.proxy_jump = Some("bastion".into()),
        ];
        for edit in edits {
            let mut edited = entry.clone();
            edit(&mut edited);
            assert_eq!(edited.id, entry.id);
            assert_eq!(
                consent(&general, &edited),
                InstallConsent::Ask,
                "{}",
                edited.dial_identity()
            );
        }
        // A "never" does not follow the entry either.
        general.set_remote_daemon_install(
            entry.id,
            &entry.dial_identity(),
            RemoteDaemonInstall::Never,
        );
        assert_eq!(consent(&general, &entry), InstallConsent::Never);
        let mut moved = entry.clone();
        moved.hostname = "prod-b.example".into();
        assert_eq!(consent(&general, &moved), InstallConsent::Ask);
    }

    // SDTEST-3050 (SDUC-574): a failed install reaches the user as a
    // sentence of the interface language, never as the English detail of
    // the error or the name of an enum variant. Every cause has copy in
    // both shipped locales.
    #[test]
    fn sdtest_3050_every_install_failure_has_copy_in_both_locales() {
        use super::{install_failure_key, InstallFailureKind};
        use shelldeck_ssh::sessiond_install::FallbackReason;
        use std::collections::BTreeSet;

        let reasons = [
            FallbackReason::ProbeFailed,
            FallbackReason::UnsupportedPlatform,
            FallbackReason::NoArtifact,
            FallbackReason::ConsentPending,
            FallbackReason::ConsentDeclined,
            FallbackReason::InstallFailed,
            FallbackReason::Unavailable,
            FallbackReason::VersionMismatch,
            FallbackReason::ConnectFailed,
        ];
        let kinds = [
            InstallFailureKind::NoArtifact,
            InstallFailureKind::Checksum,
            InstallFailureKind::NoChecksumTool,
            InstallFailureKind::CannotExecute,
            InstallFailureKind::Timeout,
            InstallFailureKind::Connection,
            InstallFailureKind::Other,
        ]
        .into_iter()
        .chain(reasons.into_iter().map(InstallFailureKind::Fallback));
        let keys: Vec<&str> = kinds.map(install_failure_key).collect();
        assert_eq!(
            keys.iter().collect::<BTreeSet<_>>().len(),
            keys.len(),
            "two causes must not share one sentence"
        );
        for (locale, source) in [
            (
                "fr",
                include_str!("../../../shelldeck-core/locales/fr.toml"),
            ),
            (
                "en",
                include_str!("../../../shelldeck-core/locales/en.toml"),
            ),
        ] {
            let table: toml::Table = toml::from_str(source).expect(locale);
            for key in &keys {
                // The locale files use unquoted dotted keys: TOML nests them.
                let mut segments = key.split('.');
                let last = segments.next_back().expect("a key has segments");
                let copy = segments
                    .try_fold(&table, |table, segment| table.get(segment)?.as_table())
                    .and_then(|table| table.get(last)?.as_str())
                    .unwrap_or_else(|| panic!("{locale}: {key} is missing"));
                assert!(!copy.trim().is_empty(), "{locale}: {key} is empty");
            }
        }
    }

    fn pane(label: &str, alive: bool) -> PaneInfo {
        PaneInfo {
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
            clients: 0,
            offset: 0,
        }
    }

    // SDTEST-3021 (SDUC-574): which remote panes a connecting tab takes
    // and which come back as tabs of their own.
    #[test]
    fn sdtest_3021_remote_panes_are_offered_back_once() {
        let (a, b, c) = (
            pane(PANE_LABEL, true),
            pane(PANE_LABEL, true),
            pane(PANE_LABEL, true),
        );
        let dead = pane(PANE_LABEL, false);
        let foreign = pane("someone-else", true);
        let listed = [
            dead.clone(),
            a.clone(),
            foreign.clone(),
            b.clone(),
            c.clone(),
        ];

        // Nothing on the host: the tab spawns a new pane.
        assert_eq!(
            ready_plan(&[dead.clone(), foreign.clone()], &[], false),
            ReadyPlan {
                tab: None,
                extra_tabs: vec![]
            }
        );
        // One tab connecting, three panes left by an earlier run: the tab
        // takes the first, the others come back as tabs, in the daemon's
        // order. Dead panes and panes of other tools never do.
        assert_eq!(
            ready_plan(&listed, &[], false),
            ReadyPlan {
                tab: Some(a.pane),
                extra_tabs: vec![b.pane, c.pane]
            }
        );
        // Three restored tabs connecting at once: each takes one pane and
        // none of them duplicates the others.
        assert_eq!(
            ready_plan(&listed, &[], true),
            ReadyPlan {
                tab: Some(a.pane),
                extra_tabs: vec![]
            }
        );
        assert_eq!(
            ready_plan(&listed, &[a.pane], true),
            ReadyPlan {
                tab: Some(b.pane),
                extra_tabs: vec![]
            }
        );
        assert_eq!(
            ready_plan(&listed, &[a.pane, b.pane], false),
            ReadyPlan {
                tab: Some(c.pane),
                extra_tabs: vec![]
            }
        );
        // Panes another client shows right now (the app on another
        // machine) are left to it: not taken, not offered as tabs.
        let mut theirs = pane(PANE_LABEL, true);
        theirs.clients = 1;
        let mut also_theirs = pane(PANE_LABEL, true);
        also_theirs.clients = 2;
        assert_eq!(
            ready_plan(&[theirs.clone(), also_theirs.clone()], &[], false),
            ReadyPlan {
                tab: None,
                extra_tabs: vec![]
            }
        );
        assert_eq!(
            ready_plan(&[theirs, a.clone(), also_theirs, b.clone()], &[], false),
            ReadyPlan {
                tab: Some(a.pane),
                extra_tabs: vec![b.pane]
            }
        );
        // Every pane is shown already: a further tab is a new pane.
        assert_eq!(
            ready_plan(&listed, &[a.pane, b.pane, c.pane], false),
            ReadyPlan {
                tab: None,
                extra_tabs: vec![]
            }
        );
    }
}
