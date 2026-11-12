//! Warm, per-session Claude CLI processes for local runs.
//!
//! A one-shot `claude -p` per turn starts cold every time: the CLI attaches
//! claude.ai connectors (Gmail, Drive, Docs…) asynchronously, and a resumed
//! turn that sends its first request before they connect sees them *removed*
//! from its tool list. The model then reports the connector as disconnected
//! even though it worked on the previous turn.
//!
//! Instead, each local Claude session keeps one `claude -p --input-format
//! stream-json` process alive between turns. A run writes one user message to
//! its stdin and ends at the matching `result` event; the process returns to
//! the pool with its MCP connections intact. A freshly spawned process waits
//! (bounded) until no MCP server reports `pending` before the first message,
//! so the very first turn sees the connectors as well.
//!
//! Lifetime rules:
//! - cancel kills the process (it is never reused mid-turn);
//! - a change of launch arguments (model, access, MCP files) respawns with
//!   `--resume`, so the conversation continues;
//! - idle processes are reaped after [`IDLE_TTL`];
//! - if ShellDeck exits, the CLI sees EOF on stdin and exits on its own.

use super::*;
use shelldeck_core::agent_runtime::{
    configure_local_process, terminate_local_process, AgentCommandSpec, AgentProvider,
    AgentRunRequest, AgentStreamEvent, AgentStreamFrame, AgentStreamFramer, AgentStreamParser,
    AgentTarget, LocalProcessTree, AGENT_STREAM_OVERSIZED_LABEL,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Warm processes unused for this long are terminated.
const IDLE_TTL: Duration = Duration::from_secs(15 * 60);
/// Upper bound on waiting for MCP servers to leave `pending` after a spawn.
const MCP_PREWAIT_CAP: Duration = Duration::from_secs(10);
const MCP_STATUS_INTERVAL: Duration = Duration::from_millis(400);
/// The first status replies list no servers at all, before claude.ai
/// connectors are registered. An empty list only counts as settled after this.
const MCP_EMPTY_GRACE: Duration = Duration::from_secs(3);
/// At most this many idle warm processes are kept; the oldest go first.
const MAX_WARM: usize = 6;

enum Output {
    Stdout(AgentStreamFrame),
    Stderr(AgentStreamFrame),
}

struct WarmClaude {
    child: Child,
    tree: LocalProcessTree,
    stdin: ChildStdin,
    output: Receiver<Output>,
    fingerprint: Vec<String>,
    last_used: Instant,
}

impl WarmClaude {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(mut self) {
        let _ = terminate_local_process(&mut self.child, &mut self.tree);
    }

    fn write_line(&mut self, value: &serde_json::Value) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        self.stdin.write_all(&line)?;
        self.stdin.flush()
    }
}

fn pool() -> &'static parking_lot::Mutex<HashMap<String, WarmClaude>> {
    static POOL: OnceLock<parking_lot::Mutex<HashMap<String, WarmClaude>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("agent-claude-warm-reaper".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(60));
                reap_idle();
            });
        parking_lot::Mutex::new(HashMap::new())
    })
}

fn reap_idle() {
    let expired: Vec<WarmClaude> = {
        let mut pool = pool().lock();
        let keys: Vec<String> = pool
            .iter()
            .filter(|(_, warm)| warm.last_used.elapsed() >= IDLE_TTL)
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| pool.remove(&key))
            .collect()
    };
    for warm in expired {
        warm.kill();
    }
}

fn put_back(key: String, mut warm: WarmClaude) {
    warm.last_used = Instant::now();
    let evicted: Vec<WarmClaude> = {
        let mut pool = pool().lock();
        let mut evicted: Vec<WarmClaude> = pool.insert(key, warm).into_iter().collect();
        while pool.len() > MAX_WARM {
            let Some(oldest) = pool
                .iter()
                .min_by_key(|(_, warm)| warm.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            evicted.extend(pool.remove(&oldest));
        }
        evicted
    };
    for warm in evicted {
        warm.kill();
    }
}

/// Whether a run can use a warm process. Run-owned private files (browser MCP
/// configs) and image attachments are bound to one run, so those keep the
/// one-shot path.
pub(super) fn eligible(
    request: &AgentRunRequest,
    options: &shelldeck_core::agent_runtime::AgentInputOptions,
) -> bool {
    request.provider == AgentProvider::Claude
        && matches!(request.target, AgentTarget::Local)
        && options.mcp_servers.is_empty()
        && options.image_paths.is_empty()
}

/// The native Claude session this run belongs to. A new session is started
/// with `--session-id <run id>`, and later turns resume that same id.
pub(super) fn session_key(request: &AgentRunRequest) -> String {
    request
        .resume_session
        .clone()
        .unwrap_or_else(|| request.id.to_string())
}

/// Launch arguments without the session selector, plus program and cwd: two
/// runs with the same fingerprint can share a process.
fn fingerprint(spec: &AgentCommandSpec) -> Vec<String> {
    let mut out = vec![spec.program.clone(), spec.cwd.display().to_string()];
    let mut args = spec.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--session-id" || arg == "--resume" {
            args.next();
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn streaming_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 2);
    for arg in args {
        out.push(arg.clone());
        if arg == "-p" {
            out.push("--input-format".into());
            out.push("stream-json".into());
        }
    }
    out
}

fn spawn_reader(
    mut reader: impl Read + Send + 'static,
    stdout: bool,
    tx: Sender<Output>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("agent-claude-warm-io".into())
        .spawn(move || {
            let mut framer = AgentStreamFramer::default();
            let mut chunk = [0_u8; 8 * 1024];
            let wrap = |frame| {
                if stdout {
                    Output::Stdout(frame)
                } else {
                    Output::Stderr(frame)
                }
            };
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        for frame in framer.push(&chunk[..read]) {
                            if tx.send(wrap(frame)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            for frame in framer.finish() {
                let _ = tx.send(wrap(frame));
            }
        })
        .map(|_| ())
}

fn spawn_process(spec: &AgentCommandSpec) -> std::io::Result<WarmClaude> {
    let mut command = Command::new(&spec.program);
    command
        .args(streaming_args(&spec.args))
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in &spec.remove_env {
        command.env_remove(key);
    }
    configure_local_process(&mut command);
    let mut child = command.spawn()?;
    let mut tree = match LocalProcessTree::capture(child.id()) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let (tx, output) = std::sync::mpsc::channel();
    let setup = (|| {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("no stderr"))?;
        spawn_reader(stdout, true, tx.clone())?;
        spawn_reader(stderr, false, tx)?;
        Ok::<_, std::io::Error>(stdin)
    })();
    match setup {
        Ok(stdin) => Ok(WarmClaude {
            child,
            tree,
            stdin,
            output,
            fingerprint: fingerprint(spec),
            last_used: Instant::now(),
        }),
        Err(error) => {
            let _ = terminate_local_process(&mut child, &mut tree);
            Err(error)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum McpStatus {
    /// No server listed yet (or the user has none).
    Empty,
    Pending,
    Settled,
}

/// Classify an `mcp_status` control response. `None` when the response is
/// not a usable status (older CLI, error).
fn mcp_status(value: &serde_json::Value) -> Option<McpStatus> {
    let response = &value["response"];
    if response["subtype"].as_str() != Some("success") {
        return None;
    }
    let servers = response["response"]["mcpServers"].as_array()?;
    Some(if servers.is_empty() {
        McpStatus::Empty
    } else if servers
        .iter()
        .any(|server| server["status"].as_str() == Some("pending"))
    {
        McpStatus::Pending
    } else {
        McpStatus::Settled
    })
}

/// After a spawn, wait until MCP servers have settled (bounded). Returns
/// `false` if the run was cancelled or the process went away meanwhile.
fn prewait_mcp(
    warm: &mut WarmClaude,
    shutdown: &mut tokio::sync::mpsc::Receiver<()>,
    events: &Sender<AgentStreamEvent>,
) -> bool {
    let started = Instant::now();
    let mut next_id = 0_u64;
    let mut send = |warm: &mut WarmClaude, subtype: &str| {
        next_id += 1;
        warm.write_line(&serde_json::json!({
            "type": "control_request",
            "request_id": format!("shelldeck-warm-{next_id}"),
            "request": { "subtype": subtype },
        }))
    };
    if send(warm, "initialize").is_err() {
        return false;
    }
    let mut announced = false;
    let mut last_poll = Instant::now() - MCP_STATUS_INTERVAL;
    while started.elapsed() < MCP_PREWAIT_CAP {
        if !matches!(
            shutdown.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ) {
            return false;
        }
        if last_poll.elapsed() >= MCP_STATUS_INTERVAL {
            if send(warm, "mcp_status").is_err() {
                return false;
            }
            last_poll = Instant::now();
        }
        match warm.output.recv_timeout(Duration::from_millis(100)) {
            Ok(Output::Stdout(AgentStreamFrame::Line(line))) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if value["type"].as_str() != Some("control_response") {
                    continue;
                }
                let id = value["response"]["request_id"].as_str().unwrap_or("");
                if id.ends_with("-1") {
                    // The initialize reply; an error means no control channel.
                    if value["response"]["subtype"].as_str() == Some("error") {
                        return true;
                    }
                    continue;
                }
                match mcp_status(&value) {
                    Some(McpStatus::Settled) | None => return true,
                    Some(McpStatus::Empty) if started.elapsed() >= MCP_EMPTY_GRACE => return true,
                    Some(McpStatus::Empty) => {}
                    Some(McpStatus::Pending) if !announced => {
                        announced = true;
                        let _ = events.send(AgentStreamEvent::Activity(
                            "Connecting MCP servers…".to_string(),
                        ));
                    }
                    Some(McpStatus::Pending) => {}
                }
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
    true
}

/// Run one Claude turn on a warm (or freshly spawned) local process. Same
/// contract as `spawn_local_agent`: stream events on `event_tx`, exactly one
/// completion on `done_tx`, and `shutdown_rx` cancels.
pub(super) fn spawn_local_claude_warm(
    run_id: Uuid,
    key: String,
    spec: AgentCommandSpec,
    prompt: String,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    event_tx: Sender<AgentStreamEvent>,
    done_tx: Sender<super::agents::AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("agent-claude-warm-{run_id}"))
        .spawn(move || {
            let wanted = fingerprint(&spec);
            let reusable = pool().lock().remove(&key).map(|mut warm| {
                let usable = warm.fingerprint == wanted && warm.alive();
                (warm, usable)
            });
            let mut warm = match reusable {
                Some((warm, true)) => warm,
                stale => {
                    if let Some((stale, _)) = stale {
                        stale.kill();
                    }
                    match spawn_process(&spec) {
                        Ok(mut warm) => {
                            if !prewait_mcp(&mut warm, &mut shutdown_rx, &event_tx) {
                                let cancelled = matches!(
                                    shutdown_rx.try_recv(),
                                    Ok(())
                                        | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
                                );
                                let exit = warm.child.try_wait().ok().flatten();
                                warm.kill();
                                let _ = done_tx.send(match (cancelled, exit) {
                                    (true, _) => (None, None),
                                    (false, Some(status)) => (status.code(), None),
                                    (false, None) => {
                                        (None, Some("Claude exited during startup".into()))
                                    }
                                });
                                return;
                            }
                            warm
                        }
                        Err(error) => {
                            let _ = done_tx.send((None, Some(error.to_string())));
                            return;
                        }
                    }
                }
            };

            // Discard anything left over between turns (late control replies).
            while warm.output.try_recv().is_ok() {}

            let message = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": prompt },
            });
            if let Err(error) = warm.write_line(&message) {
                warm.kill();
                let _ = done_tx.send((None, Some(error.to_string())));
                return;
            }

            let mut parser = AgentStreamParser::new(AgentProvider::Claude);
            let finish_parser = |parser: &mut AgentStreamParser| {
                for event in parser.finish() {
                    let _ = event_tx.send(event);
                }
            };
            loop {
                match shutdown_rx.try_recv() {
                    Ok(()) | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        warm.kill();
                        finish_parser(&mut parser);
                        let _ = done_tx.send((None, None));
                        return;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
                match warm.output.recv_timeout(Duration::from_millis(50)) {
                    Ok(Output::Stdout(AgentStreamFrame::Line(line))) => {
                        let value = serde_json::from_str::<serde_json::Value>(&line).ok();
                        let kind = value
                            .as_ref()
                            .and_then(|value| value["type"].as_str())
                            .unwrap_or("");
                        if kind == "control_response" {
                            continue;
                        }
                        for event in parser.parse_line(&line) {
                            let _ = event_tx.send(event);
                        }
                        if kind == "result" {
                            let failed = value
                                .as_ref()
                                .and_then(|value| value["is_error"].as_bool())
                                .unwrap_or(false);
                            finish_parser(&mut parser);
                            let _ = done_tx.send((Some(if failed { 1 } else { 0 }), None));
                            if warm.alive() {
                                put_back(key, warm);
                            } else {
                                warm.kill();
                            }
                            return;
                        }
                    }
                    Ok(Output::Stdout(AgentStreamFrame::Oversized))
                    | Ok(Output::Stderr(AgentStreamFrame::Oversized)) => {
                        let _ = event_tx.send(AgentStreamEvent::Activity(
                            AGENT_STREAM_OVERSIZED_LABEL.to_string(),
                        ));
                    }
                    Ok(Output::Stderr(AgentStreamFrame::Line(line))) => {
                        if !line.trim().is_empty() {
                            let _ = event_tx.send(AgentStreamEvent::Activity(line));
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        // Both pipes closed: the CLI exited before `result`.
                        finish_parser(&mut parser);
                        let status = warm.child.wait().ok().and_then(|status| status.code());
                        warm.tree.disarm();
                        let _ = done_tx.send((status, None));
                        return;
                    }
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{
        fingerprint, mcp_status, pool, session_key, spawn_local_claude_warm, streaming_args,
        AgentCommandSpec, AgentProvider, AgentRunRequest, AgentStreamEvent, AgentTarget, Instant,
        McpStatus,
    };
    use std::path::PathBuf;
    use uuid::Uuid;

    fn spec(args: &[&str]) -> AgentCommandSpec {
        AgentCommandSpec {
            program: "claude".into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: PathBuf::from("/work"),
            stdin: None,
            remove_env: Vec::new(),
        }
    }

    #[test]
    fn first_turn_and_resumed_turn_share_a_fingerprint() {
        let first = spec(&["-p", "--permission-mode", "plan", "--session-id", "abc"]);
        let resumed = spec(&["-p", "--permission-mode", "plan", "--resume", "abc"]);
        assert_eq!(fingerprint(&first), fingerprint(&resumed));
        let other_mode = spec(&["-p", "--permission-mode", "acceptEdits", "--resume", "abc"]);
        assert_ne!(fingerprint(&first), fingerprint(&other_mode));
    }

    #[test]
    fn streaming_input_follows_print_flag() {
        let args = streaming_args(&["-p".into(), "--verbose".into()]);
        assert_eq!(args, ["-p", "--input-format", "stream-json", "--verbose"]);
    }

    #[test]
    fn mcp_status_pending_detection() {
        let parse = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        let pending = parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"x","response":{"mcpServers":[{"name":"claude.ai Gmail","status":"pending"},{"name":"d","status":"needs-auth"}]}}}"#,
        );
        assert_eq!(mcp_status(&pending), Some(McpStatus::Pending));
        let settled = parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"x","response":{"mcpServers":[{"name":"claude.ai Gmail","status":"connected"},{"name":"d","status":"needs-auth"}]}}}"#,
        );
        assert_eq!(mcp_status(&settled), Some(McpStatus::Settled));
        let error = parse(
            r#"{"type":"control_response","response":{"subtype":"error","request_id":"x","error":"unknown"}}"#,
        );
        assert_eq!(mcp_status(&error), None);
        let empty = parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"x","response":{"mcpServers":[]}}}"#,
        );
        assert_eq!(mcp_status(&empty), Some(McpStatus::Empty));
    }

    #[test]
    fn session_key_is_stable_across_turns() {
        let mut request = AgentRunRequest::new(
            AgentProvider::Claude,
            AgentTarget::Local,
            shelldeck_core::agent_runtime::AgentAccessMode::ReadOnly,
            "/work".to_string(),
            None,
            "hi",
        );
        let first = session_key(&request);
        assert_eq!(first, request.id.to_string());
        request.id = Uuid::new_v4();
        request.resume_session = Some(first.clone());
        assert_eq!(session_key(&request), first);
    }

    /// Real CLI, real subscription: `cargo test -p shelldeck-ui --lib
    /// warm_claude_keeps_connectors -- --ignored --nocapture`. Requires a
    /// logged-in `claude` with at least one claude.ai connector.
    #[test]
    #[ignore]
    fn warm_claude_keeps_connectors_across_turns() {
        let program = super::super::agents::resolve_local_agent_program("claude").expect("claude");
        let session = Uuid::new_v4().to_string();
        let question = "Without calling any tool, reply with only the number of tool names you \
                        were told about (deferred included) whose name starts with mcp__claude_ai_.";
        let mut answers = Vec::new();
        for turn in 0..2 {
            let session_args = if turn == 0 {
                "--session-id"
            } else {
                "--resume"
            };
            let spec = AgentCommandSpec {
                program: program.clone(),
                args: [
                    "-p",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-mode",
                    "plan",
                    session_args,
                    &session,
                ]
                .iter()
                .map(|arg| arg.to_string())
                .collect(),
                cwd: std::env::temp_dir(),
                stdin: None,
                remove_env: Vec::new(),
            };
            let (_shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
            let (event_tx, event_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let started = Instant::now();
            spawn_local_claude_warm(
                Uuid::new_v4(),
                session.clone(),
                spec,
                question.into(),
                shutdown_rx,
                event_tx,
                done_tx,
            )
            .unwrap()
            .join()
            .unwrap();
            let done = done_rx.recv().unwrap();
            let text: String = event_rx
                .try_iter()
                .filter_map(|event| match event {
                    AgentStreamEvent::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            println!(
                "turn {turn}: done={done:?} in {:?} text={text:?}",
                started.elapsed()
            );
            assert_eq!(done, (Some(0), None));
            answers.push(text);
        }
        assert_eq!(pool().lock().len(), 1, "process returned to the pool");
        for answer in &answers {
            let count: u32 = answer.trim().trim_matches('.').parse().unwrap_or(0);
            assert!(count > 0, "connectors missing: {answers:?}");
        }
    }
}
