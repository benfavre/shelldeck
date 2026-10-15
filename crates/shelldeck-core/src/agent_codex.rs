//! Local, bidirectional Codex app-server transport (stdio JSON-RPC).
//!
//! Protocol: <https://learn.chatgpt.com/docs/app-server>. Wire fields were
//! checked against `codex app-server generate-json-schema --experimental`.
//! The user's existing Codex authentication/configuration stays owned by the
//! CLI. ShellDeck neither reads credentials nor supplies a replacement token.

use crate::agent_runtime::*;
use crate::{Result, ShellDeckError};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

mod events;
pub mod model_discovery;
mod process;

const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const INTERACTION_TIMEOUT: Duration = Duration::from_secs(180);
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PENDING_REQUESTS: usize = 32;
const MAX_RUN_TIME: Duration = Duration::from_secs(24 * 60 * 60);

/// Commands are scoped to this run's thread and active turn. Approval IDs are
/// opaque local routing IDs, never an instruction to grant a general policy.
#[derive(Clone)]
pub enum LiveCodexCommand {
    Steer {
        text: String,
    },
    Interrupt,
    Approval {
        request_id: String,
        option_id: Option<String>,
    },
    Answer {
        question_id: String,
        answer: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveCodexOutcome {
    Completed,
    Interrupted,
}

pub struct LiveCodexRun {
    /// Dropping all command senders cancels and terminates the owned process.
    pub commands: SyncSender<LiveCodexCommand>,
    /// Priority stop bypasses a full steering queue. The worker owns cleanup.
    pub cancel: Arc<AtomicBool>,
    pub join: JoinHandle<Result<LiveCodexOutcome>>,
}

/// Start only after the UI has confirmed the selected run-level access. There
/// is deliberately no fallback to `codex exec` after a possibly accepted turn.
pub fn spawn_codex_run(
    request: AgentRunRequest,
    options: AgentInputOptions,
    program: PathBuf,
    events: Sender<AgentStreamEvent>,
) -> Result<LiveCodexRun> {
    validate(&request, &options)?;
    let (commands, receiver) = mpsc::sync_channel(32);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let join = std::thread::Builder::new()
        .name("shelldeck-codex".into())
        .spawn(move || {
            let mut wire = process::ProcessWire::spawn(&program, &request.workdir)?;
            let result = drive(
                &request,
                &options,
                &mut wire,
                &receiver,
                &events,
                &worker_cancel,
            );
            // The guard kills descendants even when app-server completed or
            // exited while a child still retains a pipe. No detached executor.
            drop(wire);
            if worker_cancel.load(Ordering::Acquire) {
                Ok(LiveCodexOutcome::Interrupted)
            } else {
                result
            }
        })
        .map_err(|_| failure("Could not start the Codex transport worker"))?;
    Ok(LiveCodexRun {
        commands,
        cancel,
        join,
    })
}

fn failure(message: &str) -> ShellDeckError {
    ShellDeckError::Config(message.to_string())
}

fn safe(text: &str) -> String {
    bounded_agent_context_text(text)
}

fn validate(request: &AgentRunRequest, options: &AgentInputOptions) -> Result<()> {
    request.validate()?;
    if request.provider != AgentProvider::Codex || request.target != AgentTarget::Local {
        return Err(failure(
            "Interactive Codex transport requires a local Codex target",
        ));
    }
    // Reuse the admission rules for selected-target absolute image paths and
    // supported effort settings without executing the resulting CLI command.
    AgentCommandSpec::for_request_with_options(request, options)?;
    if request.workdir.contains(['\0', '\r', '\n']) {
        return Err(failure("Invalid Codex working directory"));
    }
    Ok(())
}

/// Injectable boundary: contract tests use in-memory pipes, never a real CLI
/// process, subscription request, model inference, shell, or network call.
trait CodexWire {
    fn send(&mut self, message: Value) -> Result<()>;
    fn receive(&mut self, timeout: Duration) -> Result<Option<Value>>;
}

struct Rpc {
    next_id: u64,
    pending: BTreeMap<u64, PendingRpc>,
}
struct PendingRpc {
    at: Instant,
    user_text: Option<String>,
}
impl Rpc {
    fn new() -> Self {
        Self {
            next_id: 1,
            pending: BTreeMap::new(),
        }
    }
    fn request(&mut self, wire: &mut impl CodexWire, method: &str, params: Value) -> Result<u64> {
        if self.pending.len() >= MAX_PENDING_REQUESTS {
            return Err(failure("Too many unanswered Codex requests"));
        }
        let id = self.next_id;
        self.next_id += 1;
        wire.send(json!({"id":id,"method":method,"params":params}))?;
        self.pending.insert(
            id,
            PendingRpc {
                at: Instant::now(),
                user_text: None,
            },
        );
        Ok(id)
    }
    fn wait(
        &mut self,
        wire: &mut impl CodexWire,
        id: u64,
        commands: &Receiver<LiveCodexCommand>,
        cancel: &AtomicBool,
    ) -> Result<Value> {
        let deadline = Instant::now() + RPC_TIMEOUT;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(failure("Codex startup was cancelled"));
            }
            match commands.try_recv() {
                Ok(LiveCodexCommand::Interrupt) | Err(TryRecvError::Disconnected) => {
                    return Err(failure("Codex startup was cancelled"));
                }
                Ok(_) => return Err(failure("Codex is still initializing; input was not sent")),
                Err(TryRecvError::Empty) => {}
            }
            if Instant::now() >= deadline {
                return Err(failure("Codex initialization timed out"));
            }
            let Some(message) = wire.receive(Duration::from_millis(50))? else {
                continue;
            };
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    reject_unsupported(wire, &message)?;
                }
                continue;
            }
            if message["id"].as_u64() != Some(id) {
                return Err(failure("Unexpected Codex initialization response"));
            }
            self.pending.remove(&id);
            return rpc_result(&message);
        }
    }
}

fn rpc_result(message: &Value) -> Result<Value> {
    if let Some(error) = message.get("error") {
        let detail = error["message"].as_str().unwrap_or("Codex request failed");
        return Err(failure(&safe(detail)));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| failure("Malformed Codex response"))
}

fn emit(events: &Sender<AgentStreamEvent>, event: AgentStreamEvent) -> Result<()> {
    events
        .send(event)
        .map_err(|_| failure("Codex event consumer disconnected"))
}

fn sandbox(access: AgentAccessMode) -> Value {
    match access {
        AgentAccessMode::ReadOnly => json!({"type":"readOnly","networkAccess":false}),
        AgentAccessMode::WorkspaceWrite => json!({"type":"workspaceWrite","networkAccess":false,
            "excludeTmpdirEnvVar":true,"excludeSlashTmp":true}),
        AgentAccessMode::FullAccess => json!({"type":"dangerFullAccess"}),
    }
}
fn approval_policy(access: AgentAccessMode) -> &'static str {
    if access == AgentAccessMode::ReadOnly {
        "never"
    } else {
        "on-request"
    }
}

fn input(text: &str, options: &AgentInputOptions) -> Vec<Value> {
    let mut parts = vec![json!({"type":"text","text":text})];
    parts.extend(
        options
            .image_paths
            .iter()
            .map(|path| json!({"type":"localImage","path":path.to_string_lossy()})),
    );
    parts
}

fn drive(
    request: &AgentRunRequest,
    options: &AgentInputOptions,
    wire: &mut impl CodexWire,
    commands: &Receiver<LiveCodexCommand>,
    events: &Sender<AgentStreamEvent>,
    cancel: &AtomicBool,
) -> Result<LiveCodexOutcome> {
    let mut rpc = Rpc::new();
    let init = rpc.request(
        wire,
        "initialize",
        json!({"clientInfo": {
        "name":"shelldeck","title":"ShellDeck","version":env!("CARGO_PKG_VERSION")},
        "capabilities":{"experimentalApi":true}}),
    )?;
    rpc.wait(wire, init, commands, cancel)?;
    wire.send(json!({"method":"initialized"}))?;
    let sandbox_name = match request.access {
        AgentAccessMode::ReadOnly => "read-only",
        AgentAccessMode::WorkspaceWrite => "workspace-write",
        AgentAccessMode::FullAccess => "danger-full-access",
    };
    let mut params = json!({"cwd":request.workdir.trim(),"model":request.model,
        "sandbox":sandbox_name,"approvalPolicy":approval_policy(request.access),
        "approvalsReviewer":"user"});
    if !options.mcp_servers.is_empty() {
        params["config"] = crate::agent_browser_tools::codex_browser_config(&options.mcp_servers);
    }
    let method = if let Some(resume) = &request.resume_session {
        params["threadId"] = json!(resume);
        "thread/resume"
    } else {
        "thread/start"
    };
    let thread_request = rpc.request(wire, method, params)?;
    let result = rpc.wait(wire, thread_request, commands, cancel)?;
    let thread_id = valid_id(&result["thread"]["id"])?;
    if request
        .resume_session
        .as_ref()
        .is_some_and(|id| id != &thread_id)
    {
        return Err(failure("Codex resumed an unexpected conversation"));
    }
    // Refuse unexpected inherited permissions before any turn is submitted.
    let expected_type = sandbox(request.access)["type"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if result.pointer("/sandbox/type").and_then(Value::as_str) != Some(expected_type.as_str()) {
        return Err(failure("Codex did not confirm the selected sandbox"));
    }
    let actual_cwd = result["cwd"]
        .as_str()
        .ok_or_else(|| failure("Codex did not confirm its working directory"))?;
    if !same_directory(actual_cwd, request.workdir.trim()) {
        return Err(failure("Codex returned a different working directory"));
    }
    emit(events, AgentStreamEvent::Session(thread_id.clone()))?;
    emit(
        events,
        AgentStreamEvent::Metadata(
            AgentRunMetadata {
                model: result["model"].as_str().map(safe),
                provider_version: result
                    .pointer("/thread/cliVersion")
                    .and_then(Value::as_str)
                    .map(safe),
                ..Default::default()
            }
            .bounded(),
        ),
    )?;
    let mut turn_params = json!({"threadId":thread_id,"input":input(&request.prompt, options),
        "cwd":request.workdir.trim(),"model":request.model,
        "sandboxPolicy":sandbox(request.access),"approvalPolicy":approval_policy(request.access),
        "approvalsReviewer":"user"});
    if let Some(effort) = options.reasoning_effort {
        turn_params["effort"] = json!(effort.as_str());
    }
    let turn_start = rpc.request(wire, "turn/start", turn_params)?;
    // turn/started and item notifications may precede the turn/start response;
    // let the active loop preserve those rather than discarding startup work.
    let mut state = RunState::new(thread_id, request.access, turn_start);
    run_active(wire, commands, events, &mut rpc, &mut state, cancel)
}

fn same_directory(left: &str, right: &str) -> bool {
    Path::new(left) == Path::new(right)
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

fn valid_id(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= MAX_AGENT_SESSION_BYTES
                && !id.contains(['\0', '\r', '\n'])
        })
        .map(str::to_string)
        .ok_or_else(|| failure("Invalid Codex protocol identifier"))
}

fn server_id(message: &Value) -> Result<Value> {
    let value = &message["id"];
    if value.as_i64().is_some() || value.as_u64().is_some() {
        return Ok(value.clone());
    }
    valid_id(value)?;
    Ok(value.clone())
}

fn reject_unsupported(wire: &mut impl CodexWire, message: &Value) -> Result<()> {
    wire.send(json!({"id":server_id(message)?,"error":{"code":-32601,
        "message":"ShellDeck does not support this interactive request"}}))
}

#[path = "agent_codex/interactions.rs"]
mod interactions;
use interactions::*;

#[cfg(test)]
#[path = "agent_codex/tests.rs"]
mod tests;
