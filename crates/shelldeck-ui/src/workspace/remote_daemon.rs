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
use shelldeck_core::config::app_config::{GeneralConfig, RemoteDaemonInstall};
use shelldeck_core::config::workspace_state::DaemonCleanup;
use shelldeck_core::config::WorkspaceState;
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

/// What is left to end on one host at the next contact with its daemon.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HostCleanup {
    /// Where the entry dialed when the work was recorded.
    target: String,
    /// Panes closed while the host could not be reached, in an earlier
    /// run (the supervisor of the host holds the ones of this run).
    panes: Vec<Uuid>,
    /// Sign-out while the host was not connected: end every shell the
    /// app left there.
    all: bool,
}

struct State {
    /// Work carried from earlier runs (the workspace state file) and
    /// sign-outs of this one, by connection id.
    cleanup: HashMap<Uuid, HostCleanup>,
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
            cleanup: saved_cleanup(),
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

/// The cleanup work an earlier run left in the workspace state file.
fn saved_cleanup() -> HashMap<Uuid, HostCleanup> {
    WorkspaceState::load()
        .map(|state| state.daemon_cleanup)
        .unwrap_or_default()
        .into_iter()
        .map(|saved| {
            (
                saved.connection_id,
                HostCleanup {
                    target: saved.target,
                    panes: saved.panes,
                    all: saved.all,
                },
            )
        })
        .collect()
}

/// What goes into the workspace state file. `carried`: the work known
/// from earlier runs and sign-outs. `live`: for each host made in this
/// run, where it dials and the panes its supervisor still has to end
/// (they include the carried ones of that host that are not done yet).
/// Work recorded for a server an entry no longer dials is dropped: that
/// server cannot be reached through the entry any more.
fn cleanup_snapshot(
    carried: &HashMap<Uuid, HostCleanup>,
    live: &[(Uuid, String, Vec<Uuid>)],
) -> Vec<DaemonCleanup> {
    let mut saved: Vec<DaemonCleanup> = Vec::new();
    for (connection_id, target, doomed) in live {
        let all = carried
            .get(connection_id)
            .is_some_and(|cleanup| cleanup.all && cleanup.target == *target);
        saved.push(DaemonCleanup {
            connection_id: *connection_id,
            target: target.clone(),
            panes: doomed.clone(),
            all,
        });
    }
    for (connection_id, cleanup) in carried {
        if live.iter().all(|(id, _, _)| id != connection_id) {
            saved.push(DaemonCleanup {
                connection_id: *connection_id,
                target: cleanup.target.clone(),
                panes: cleanup.panes.clone(),
                all: cleanup.all,
            });
        }
    }
    saved.retain(|cleanup| cleanup.all || !cleanup.panes.is_empty());
    saved.sort_by_key(|cleanup| cleanup.connection_id);
    saved
}

/// The cleanup work to write to the workspace state file.
pub(super) fn cleanup_to_save() -> Vec<DaemonCleanup> {
    let state = state().lock();
    let live: Vec<(Uuid, String, Vec<Uuid>)> = state
        .hosts
        .iter()
        .map(|(connection_id, entry)| {
            (
                *connection_id,
                entry.target.clone(),
                entry.host.supervisor().doomed_panes(),
            )
        })
        .collect();
    cleanup_snapshot(&state.cleanup, &live)
}

/// Sign-out: every server the user allowed the daemon on, and that is not
/// connected right now, may still run shells of this account. They are
/// ended at the next contact (`take_purge`). `ready`: is the daemon of
/// that connection reachable right now (its shells were just ended).
fn mark_cleanup_all(
    cleanup: &mut HashMap<Uuid, HostCleanup>,
    allowed: &[(Uuid, String)],
    ready: impl Fn(Uuid, &str) -> bool,
) {
    for (connection_id, target) in allowed {
        if ready(*connection_id, target) {
            continue;
        }
        let entry = cleanup.entry(*connection_id).or_default();
        if entry.target != *target {
            // Recorded for another server: not reachable through this
            // entry any more.
            entry.panes.clear();
            entry.target.clone_from(target);
        }
        entry.all = true;
    }
}

/// See [`mark_cleanup_all`].
pub(super) fn mark_signed_out(connections: &[Connection], general: &GeneralConfig) {
    let allowed: Vec<(Uuid, String)> = connections
        .iter()
        .filter(|connection| consent(general, connection) == InstallConsent::Allow)
        .map(|connection| (connection.id, dial_target(connection)))
        .collect();
    let mut state = state().lock();
    let state = &mut *state;
    let hosts = &state.hosts;
    mark_cleanup_all(&mut state.cleanup, &allowed, |connection_id, target| {
        hosts
            .get(&connection_id)
            .is_some_and(|entry| entry.target == target && entry.host.is_ready())
    });
}

/// `true` once after a sign-out that could not reach this host: the
/// shells the app left there are to be ended now.
pub(super) fn take_purge(connection_id: Uuid) -> bool {
    let mut state = state().lock();
    let state = &mut *state;
    let Some(target) = state.hosts.get(&connection_id).map(|entry| &entry.target) else {
        return false;
    };
    match state.cleanup.get_mut(&connection_id) {
        Some(cleanup) if cleanup.all && cleanup.target == *target => {
            cleanup.all = false;
            true
        }
        _ => false,
    }
}

/// For a purge: the panes to end, and the listing without them. Only
/// what `ready_plan` could offer is ended: live shells the app made, not
/// shown here, and not shown by another client right now (the app on
/// another machine with the same account on the host).
pub(super) fn split_purged(panes: Vec<PaneInfo>, open: &[Uuid]) -> (Vec<Uuid>, Vec<PaneInfo>) {
    let (ended, kept): (Vec<PaneInfo>, Vec<PaneInfo>) = panes.into_iter().partition(|info| {
        info.alive && info.label == PANE_LABEL && info.clients == 0 && !open.contains(&info.pane)
    });
    (ended.into_iter().map(|info| info.pane).collect(), kept)
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
            artifact_dirs: crate::sessiond_files::artifact_dirs(),
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
    // Panes closed in an earlier run while this server was out of reach:
    // the supervisor ends them at its first contact and never offers them.
    match state.cleanup.get_mut(&connection.id) {
        Some(cleanup) if cleanup.target == target => {
            host.supervisor().doom(std::mem::take(&mut cleanup.panes));
        }
        // Recorded for a server this entry no longer dials.
        Some(_) => {
            state.cleanup.remove(&connection.id);
        }
        None => {}
    }
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

/// The recorded answers were forgotten: ask and notify again in this run.
pub(super) fn forget_questions() {
    let mut state = state().lock();
    state.asked.clear();
    state.noticed.clear();
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
    // The shell is started by the daemon, not by sshd: without these it
    // has no `SSH_CONNECTION`, and prompts or rc files that show the host
    // name or change behaviour "when over SSH" take it for a local login.
    let mut env = vec![("TERM".to_string(), "xterm-256color".to_string())];
    env.extend(host.pane_ssh_environment());
    host.supervisor()
        .open(
            PaneTarget::Spawn(SpawnOptions {
                label: PANE_LABEL.into(),
                // Empty argv and cwd: the account's login shell in its
                // home directory.
                env,
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

    // SDTEST-3052 (SDUC-574): the rules of the cleanup work that is kept
    // for a later contact with a server.
    //
    // Saving: a host made in this run saves what its supervisor still has
    // to end, with the pending "everything" mark when it is for the server
    // the host dials; a host not made in this run keeps what was carried;
    // an entry with nothing left is not saved.
    //
    // Sign-out: every allowed server that is not reachable right now is
    // marked "everything"; a mark recorded for a server the entry no
    // longer dials drops the old panes.
    //
    // Purge: only what could be offered back is ended (a live shell of
    // the app, not shown here, not shown by another client), and the
    // listing that remains offers nothing of it.
    #[test]
    fn sdtest_3052_cleanup_is_carried_merged_and_applied() {
        use super::{cleanup_snapshot, mark_cleanup_all, split_purged, HostCleanup};
        use std::collections::HashMap;

        let (made, away, done, moved) = (
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
        );
        let (old_pane, new_pane, away_pane) = (
            Uuid::from_u128(11),
            Uuid::from_u128(12),
            Uuid::from_u128(13),
        );
        let mut carried = HashMap::new();
        carried.insert(
            made,
            HostCleanup {
                target: "a@made:22|".into(),
                panes: Vec::new(), // given to the supervisor when the host was made
                all: true,
            },
        );
        carried.insert(
            away,
            HostCleanup {
                target: "a@away:22|".into(),
                panes: vec![away_pane],
                all: false,
            },
        );
        carried.insert(
            done,
            HostCleanup {
                target: "a@done:22|".into(),
                panes: Vec::new(),
                all: false,
            },
        );
        carried.insert(
            moved,
            HostCleanup {
                target: "a@old:22|".into(),
                panes: Vec::new(),
                all: true,
            },
        );
        let live = vec![
            (made, "a@made:22|".to_string(), vec![old_pane, new_pane]),
            (done, "a@done:22|".to_string(), Vec::new()),
            // The entry dials another server now: the mark for the old
            // one does not follow it.
            (moved, "a@new:22|".to_string(), Vec::new()),
        ];
        let saved = cleanup_snapshot(&carried, &live);
        let summary: Vec<_> = saved
            .iter()
            .map(|cleanup| {
                (
                    cleanup.connection_id,
                    cleanup.target.as_str(),
                    cleanup.panes.clone(),
                    cleanup.all,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (made, "a@made:22|", vec![old_pane, new_pane], true),
                (away, "a@away:22|", vec![away_pane], false),
            ]
        );

        // Sign-out.
        let mut cleanup = HashMap::new();
        cleanup.insert(
            away,
            HostCleanup {
                target: "a@elsewhere:22|".into(),
                panes: vec![away_pane],
                all: false,
            },
        );
        let allowed = vec![
            (made, "a@made:22|".to_string()),
            (away, "a@away:22|".to_string()),
        ];
        mark_cleanup_all(&mut cleanup, &allowed, |id, _| id == made);
        assert!(!cleanup.contains_key(&made), "reachable: ended right now");
        assert_eq!(
            cleanup.get(&away),
            Some(&HostCleanup {
                target: "a@away:22|".into(),
                panes: Vec::new(),
                all: true,
            })
        );

        // Purge.
        let shown = pane(PANE_LABEL, true);
        let left = pane(PANE_LABEL, true);
        let dead = pane(PANE_LABEL, false);
        let foreign = pane("something-else", true);
        let mut elsewhere = pane(PANE_LABEL, true);
        elsewhere.clients = 1;
        let listing = vec![
            shown.clone(),
            left.clone(),
            dead.clone(),
            foreign.clone(),
            elsewhere.clone(),
        ];
        let (ended, kept) = split_purged(listing, &[shown.pane]);
        assert_eq!(ended, vec![left.pane]);
        let kept_ids: Vec<Uuid> = kept.iter().map(|info| info.pane).collect();
        assert_eq!(
            kept_ids,
            vec![shown.pane, dead.pane, foreign.pane, elsewhere.pane]
        );
        assert_eq!(
            ready_plan(&kept, &[shown.pane], false),
            ReadyPlan {
                tab: None,
                extra_tabs: Vec::new(),
            },
            "nothing of the previous account is offered"
        );
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
