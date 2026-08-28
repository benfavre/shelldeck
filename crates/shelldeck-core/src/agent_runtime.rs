//! Provider-neutral launch contract for ShellDeck-managed coding agents.
//!
//! This is deliberately separate from [`crate::ai`]. The contextual assistant
//! is a no-tools drafting surface; an agent runtime is an explicitly selected
//! local or SSH target where a provider may inspect or mutate a workspace.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use uuid::Uuid;

use crate::{Result, ShellDeckError};

pub const MAX_AGENT_PROMPT_BYTES: usize = 48 * 1024;
pub const MAX_AGENT_MODEL_BYTES: usize = 256;
pub const MAX_AGENT_WORKDIR_BYTES: usize = 4096;
pub const MAX_AGENT_SESSION_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    Claude,
    Codex,
    /// Jcode with the provider selected by its configuration on the target.
    Jcode,
    /// DeepSeek models are launched through the managed Jcode runner. Jcode
    /// owns provider authentication and model discovery on the target host.
    DeepSeek,
}

impl AgentProvider {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Jcode => "Jcode (auto)",
            Self::DeepSeek => "DeepSeek (Jcode)",
        }
    }

    pub fn binary(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Jcode | Self::DeepSeek => "jcode",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAccessMode {
    /// No workspace-changing tools. This is the default for every new run.
    #[default]
    ReadOnly,
    /// The agent may edit and run commands inside its selected workspace.
    WorkspaceWrite,
    /// Provider-level unrestricted mode. The UI must confirm this separately.
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentTarget {
    Local,
    Ssh {
        connection_id: Uuid,
        connection_label: String,
    },
}

impl AgentTarget {
    pub fn label(&self) -> &str {
        match self {
            Self::Local => "Local",
            Self::Ssh {
                connection_label, ..
            } => connection_label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunRequest {
    pub id: Uuid,
    pub provider: AgentProvider,
    pub target: AgentTarget,
    pub access: AgentAccessMode,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session: Option<String>,
    pub prompt: String,
}

impl AgentRunRequest {
    pub fn new(
        provider: AgentProvider,
        target: AgentTarget,
        access: AgentAccessMode,
        workdir: impl Into<String>,
        model: Option<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            provider,
            target,
            access,
            workdir: workdir.into(),
            model: model.filter(|value| !value.trim().is_empty()),
            resume_session: None,
            prompt: prompt.into(),
        }
    }

    pub fn with_resume_session(mut self, session: Option<String>) -> Self {
        self.resume_session = session.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.prompt.trim().is_empty() {
            return Err(ShellDeckError::Config(
                "agent prompt must not be empty".to_string(),
            ));
        }
        if self.prompt.len() > MAX_AGENT_PROMPT_BYTES {
            return Err(ShellDeckError::Config(
                "agent prompt exceeds the safe remote-command limit".to_string(),
            ));
        }
        let workdir = self.workdir.trim();
        let absolute = match self.target {
            AgentTarget::Local => Path::new(workdir).is_absolute(),
            // Remote agent transport is POSIX shell based on every host OS.
            AgentTarget::Ssh { .. } => workdir.starts_with('/'),
        };
        if workdir.is_empty() || !absolute {
            return Err(ShellDeckError::Config(
                "agent workdir must be an absolute path".to_string(),
            ));
        }
        if self.workdir.len() > MAX_AGENT_WORKDIR_BYTES {
            return Err(ShellDeckError::Config(
                "agent workdir is too long".to_string(),
            ));
        }
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.len() > MAX_AGENT_MODEL_BYTES)
        {
            return Err(ShellDeckError::Config(
                "agent model name is too long".to_string(),
            ));
        }
        if self.resume_session.as_ref().is_some_and(|session| {
            session.trim().is_empty() || session.len() > MAX_AGENT_SESSION_BYTES
        }) {
            return Err(ShellDeckError::Config(
                "agent session identifier is invalid".to_string(),
            ));
        }
        Ok(())
    }
}

/// Process-level launch description. The caller owns process lifetime,
/// streaming, cancellation, and whether this command runs locally or over SSH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub stdin: Option<Vec<u8>>,
    pub remove_env: Vec<String>,
}

impl AgentCommandSpec {
    pub fn for_request(request: &AgentRunRequest) -> Result<Self> {
        request.validate()?;
        let cwd = PathBuf::from(request.workdir.trim());
        let model = request
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let (args, stdin, remove_env) = match request.provider {
            AgentProvider::Claude => {
                let permission_mode = match request.access {
                    AgentAccessMode::ReadOnly => "plan",
                    AgentAccessMode::WorkspaceWrite => "acceptEdits",
                    AgentAccessMode::FullAccess => "bypassPermissions",
                };
                let mut args = vec![
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--permission-mode".to_string(),
                    permission_mode.to_string(),
                ];
                if let Some(session) = &request.resume_session {
                    args.push("--resume".to_string());
                    args.push(session.clone());
                } else {
                    args.push("--session-id".to_string());
                    args.push(request.id.to_string());
                }
                push_model(&mut args, model);
                (args, Some(request.prompt.as_bytes().to_vec()), Vec::new())
            }
            AgentProvider::Codex => {
                let sandbox = match request.access {
                    AgentAccessMode::ReadOnly => "read-only",
                    AgentAccessMode::WorkspaceWrite => "workspace-write",
                    AgentAccessMode::FullAccess => "danger-full-access",
                };
                let mut args = if request.resume_session.is_some() {
                    vec![
                        "exec".to_string(),
                        "resume".to_string(),
                        "--json".to_string(),
                        "--skip-git-repo-check".to_string(),
                    ]
                } else {
                    vec![
                        "exec".to_string(),
                        "--json".to_string(),
                        "--sandbox".to_string(),
                        sandbox.to_string(),
                        "--skip-git-repo-check".to_string(),
                    ]
                };
                args.extend([
                    "-c".to_string(),
                    format!("sandbox_mode=\"{sandbox}\""),
                    "-c".to_string(),
                    "approval_policy=\"never\"".to_string(),
                ]);
                push_model(&mut args, model);
                if let Some(session) = &request.resume_session {
                    args.push(session.clone());
                }
                args.push("-".to_string());
                (args, Some(request.prompt.as_bytes().to_vec()), Vec::new())
            }
            AgentProvider::Jcode | AgentProvider::DeepSeek => {
                let mut args = vec![
                    "run".to_string(),
                    "--ndjson".to_string(),
                    "--quiet".to_string(),
                    "--no-update".to_string(),
                    "--no-selfdev".to_string(),
                    "-C".to_string(),
                    request.workdir.trim().to_string(),
                ];
                if request.provider == AgentProvider::DeepSeek {
                    args.push("--provider".to_string());
                    args.push("deepseek".to_string());
                }
                if let Some(session) = &request.resume_session {
                    args.push("--resume".to_string());
                    args.push(session.clone());
                }
                push_model(&mut args, model);
                match request.access {
                    AgentAccessMode::ReadOnly => args.extend([
                        "--disable-base-tools".to_string(),
                        "--tools".to_string(),
                        "read".to_string(),
                    ]),
                    AgentAccessMode::WorkspaceWrite => {
                        args.extend(["--tool-profile".to_string(), "minimal".to_string()])
                    }
                    AgentAccessMode::FullAccess => {
                        args.extend(["--tool-profile".to_string(), "full".to_string()])
                    }
                }
                args.push(request.prompt.clone());
                (args, None, Vec::new())
            }
        };

        Ok(Self {
            program: request.provider.binary().to_string(),
            args,
            cwd,
            stdin,
            remove_env,
        })
    }

    /// Build the command executed by `SshSession::exec_cancellable`.
    ///
    /// Every dynamic value is single-quote escaped. Prompt bytes are base64 so
    /// multiline text never becomes shell syntax; stdin-based providers decode
    /// it into a pipe, while Jcode receives the already-quoted prompt argument.
    pub fn remote_shell_command(&self) -> String {
        let env = if self.remove_env.is_empty() {
            String::new()
        } else {
            format!(
                "env {} ",
                self.remove_env
                    .iter()
                    .map(|key| format!("-u {}", shell_quote(key)))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        let command = std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ");
        let command = format!("{env}{command}");
        let command = match &self.stdin {
            Some(stdin) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(stdin);
                format!(
                    "{{ if printf '' | base64 -d >/dev/null 2>&1; then printf '%s' {0} | base64 -d; else printf '%s' {0} | base64 -D; fi; }} | {command}",
                    shell_quote(&encoded)
                )
            }
            None => command,
        };
        format!(
            "export PATH=\"$HOME/.local/bin:$HOME/bin:/usr/local/bin:/opt/homebrew/bin:$PATH\"; cd -- {} && {command}",
            shell_quote(&self.cwd.to_string_lossy())
        )
    }
}

fn push_model(args: &mut Vec<String>, model: Option<&str>) {
    if let Some(model) = model {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Prepare a local child for owned tree supervision.
///
/// Unix uses a dedicated process group. Windows starts the child suspended so
/// [`LocalProcessTree::capture`] can attach its Job Object before any tool
/// subprocess is able to start.
pub fn configure_local_process(command: &mut Command) {
    #[cfg(not(any(unix, windows)))]
    let _ = command;

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;

        // SAFETY: `setpgid` is async-signal-safe and touches no memory shared
        // with the parent. It runs after fork and before exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

        // `LocalProcessTree::capture` assigns the inert child to its Job
        // Object before resuming the primary thread. This closes the spawn to
        // assignment window in which an unowned descendant could escape.
        command.creation_flags(CREATE_SUSPENDED);
    }
}

/// Owned authority for one local subprocess tree.
///
/// Unix children are placed in a dedicated process group before `exec`.
/// Windows children are assigned immediately after spawn to a Job Object with
/// `KILL_ON_JOB_CLOSE`, so ownership survives the direct process exiting while
/// a descendant still owns its output pipes.
pub struct LocalProcessTree {
    _process_id: u32,
    armed: bool,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
// SAFETY: a Job Object HANDLE may be used and closed from any thread. This
// value owns the handle and never exposes it to callers.
unsafe impl Send for LocalProcessTree {}

impl LocalProcessTree {
    /// Capture the process tree immediately after spawning the direct child.
    pub fn capture(process_id: u32) -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
            };

            // SAFETY: all pointers are null or point to initialized values for
            // the duration of each call. Every acquired handle is closed on
            // every error path or transferred into `Self`.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error);
                }
                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, process_id);
                if process.is_null() {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error);
                }
                if AssignProcessToJobObject(job, process) == 0 {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(process);
                    CloseHandle(job);
                    return Err(error);
                }
                if let Err(error) = resume_suspended_process(process_id) {
                    windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
                    CloseHandle(process);
                    CloseHandle(job);
                    return Err(error);
                }
                CloseHandle(process);
                return Ok(Self {
                    _process_id: process_id,
                    armed: true,
                    job,
                });
            }
        }
        #[cfg(not(windows))]
        Ok(Self {
            _process_id: process_id,
            armed: true,
        })
    }

    /// Terminate every process still owned by this tree authority.
    pub fn terminate(&mut self) {
        if !self.armed {
            return;
        }
        #[cfg(unix)]
        {
            let process_group = -(self._process_id as i32);
            // SAFETY: `configure_local_process` assigned the child a process
            // group whose id is its pid; a negative pid targets only it.
            unsafe {
                libc::kill(process_group, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            // SAFETY: `job` is an owned live Job Object handle.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
            }
        }
        self.armed = false;
    }

    /// The direct child and all pipe-owning descendants have completed.
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(windows)]
fn resume_suspended_process(process_id: u32) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    // SAFETY: the snapshot and thread handles are owned here and closed on
    // every branch; `entry` has the size required by the ToolHelp contract.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut has_entry = Thread32First(snapshot, &mut entry) != 0;
        while has_entry {
            if entry.th32OwnerProcessID == process_id {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, FALSE, entry.th32ThreadID);
                if thread.is_null() {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(snapshot);
                    return Err(error);
                }
                let resumed = ResumeThread(thread);
                CloseHandle(thread);
                CloseHandle(snapshot);
                if resumed == u32::MAX {
                    return Err(std::io::Error::last_os_error());
                }
                return Ok(());
            }
            has_entry = Thread32Next(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "suspended child thread was not found",
        ))
    }
}

impl Drop for LocalProcessTree {
    fn drop(&mut self) {
        self.terminate();
        #[cfg(windows)]
        {
            // Closing a KILL_ON_JOB_CLOSE job is the final fail-safe against a
            // descendant escaping after its direct parent exited.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
    }
}

/// Terminate a local agent and its subprocess tree, then reap the CLI process.
pub fn terminate_local_process(
    child: &mut Child,
    process_tree: &mut LocalProcessTree,
) -> std::io::Result<ExitStatus> {
    process_tree.terminate();
    let _ = child.kill();
    child.wait()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStreamEvent {
    /// A complete provider message. The UI separates consecutive messages.
    Text(String),
    /// An incremental text fragment from a streaming provider.
    TextDelta(String),
    /// Opaque provider conversation identifier used only for a matching
    /// follow-up run on the same target and execution context.
    Session(String),
    /// Provider initialization completed. The UI localizes this state rather
    /// than baking a language into the provider-neutral parser.
    Ready,
    Activity(String),
    Error(String),
}

/// Normalize the useful subset of each provider's JSONL stream. Unknown
/// records become compact activity labels rather than leaking raw control
/// payloads into the transcript.
pub fn parse_stream_line(provider: AgentProvider, line: &str) -> Vec<AgentStreamEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return vec![AgentStreamEvent::Activity(trimmed.to_string())];
    };
    match provider {
        AgentProvider::Claude => parse_claude_event(&value),
        AgentProvider::Codex => parse_codex_event(&value),
        AgentProvider::Jcode | AgentProvider::DeepSeek => parse_jcode_event(&value),
    }
}

fn parse_claude_event(value: &Value) -> Vec<AgentStreamEvent> {
    let mut events = session_events(value);
    match value.get("type").and_then(Value::as_str) {
        Some("assistant")
            if value.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) =>
        {
            events.extend(
                value
                    .pointer("/message/content")
                    .and_then(text_from_value)
                    .map(AgentStreamEvent::Error),
            )
        }
        Some("assistant") => events.extend(
            value
                .pointer("/message/content")
                .and_then(text_from_value)
                .map(AgentStreamEvent::Text),
        ),
        Some("result") if value.get("is_error").and_then(Value::as_bool) == Some(true) => events
            .extend(
                value
                    .get("result")
                    .and_then(text_from_value)
                    .map(AgentStreamEvent::Error),
            ),
        Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
            events.push(AgentStreamEvent::Ready)
        }
        _ => {}
    }
    events
}

fn parse_codex_event(value: &Value) -> Vec<AgentStreamEvent> {
    let mut events = session_events(value);
    match value.get("type").and_then(Value::as_str) {
        Some("item.completed") => {
            if let Some(item) = value.get("item") {
                match item.get("type").and_then(Value::as_str) {
                    Some("agent_message") => events.extend(
                        item.get("text")
                            .and_then(text_from_value)
                            .map(AgentStreamEvent::Text),
                    ),
                    Some(kind) => events.push(AgentStreamEvent::Activity(humanize_kind(kind))),
                    None => {}
                }
            }
        }
        Some("turn.failed") | Some("error") => events.extend(
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(text_from_value)
                .map(AgentStreamEvent::Error),
        ),
        Some("thread.started") => events.push(AgentStreamEvent::Ready),
        _ => {}
    }
    events
}

fn parse_jcode_event(value: &Value) -> Vec<AgentStreamEvent> {
    let mut events = session_events(value);
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(delta) = value.get("delta").and_then(text_from_value) {
        events.push(AgentStreamEvent::TextDelta(delta));
        return events;
    }
    if matches!(kind, "result" | "assistant" | "message") {
        for key in ["result", "text", "content", "message", "response"] {
            if let Some(text) = value.get(key).and_then(text_from_value) {
                events.push(AgentStreamEvent::Text(text));
                return events;
            }
        }
    }
    if value.get("is_error").and_then(Value::as_bool) == Some(true) || kind == "error" {
        events.extend(
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(text_from_value)
                .map(AgentStreamEvent::Error),
        );
        return events;
    }
    if !kind.is_empty() {
        events.push(AgentStreamEvent::Activity(humanize_kind(kind)));
    }
    events
}

fn session_events(value: &Value) -> Vec<AgentStreamEvent> {
    ["session_id", "sessionId", "thread_id", "threadId"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .or_else(|| value.pointer("/session/id").and_then(Value::as_str))
        .filter(|session| !session.trim().is_empty() && session.len() <= MAX_AGENT_SESSION_BYTES)
        .map(|session| vec![AgentStreamEvent::Session(session.to_string())])
        .unwrap_or_default()
}

fn text_from_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }
    if let Some(items) = value.as_array() {
        let text = items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .or_else(|| item.get("content"))
                    .and_then(text_from_value)
            })
            .collect::<Vec<_>>()
            .join("");
        return (!text.is_empty()).then_some(text);
    }
    if let Some(object) = value.as_object() {
        for key in ["text", "content", "message", "result"] {
            if let Some(text) = object.get(key).and_then(text_from_value) {
                return Some(text);
            }
        }
    }
    None
}

fn humanize_kind(kind: &str) -> String {
    kind.replace(['_', '.'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    const LOCAL_TEST_WORKDIR: &str = "/srv/project";
    #[cfg(windows)]
    const LOCAL_TEST_WORKDIR: &str = r"C:\srv\project";

    fn request(provider: AgentProvider, access: AgentAccessMode, prompt: &str) -> AgentRunRequest {
        AgentRunRequest::new(
            provider,
            AgentTarget::Local,
            access,
            LOCAL_TEST_WORKDIR,
            Some("model-x".to_string()),
            prompt,
        )
    }

    // SDTEST-1673 — SDUC-475
    #[test]
    fn sdtest_1673_provider_specs_keep_access_and_prompt_boundaries() {
        let claude = AgentCommandSpec::for_request(&request(
            AgentProvider::Claude,
            AgentAccessMode::ReadOnly,
            "inspect only",
        ))
        .unwrap();
        assert!(claude
            .args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"]));
        assert_eq!(claude.stdin.as_deref(), Some("inspect only".as_bytes()));
        assert!(!claude.args.iter().any(|arg| arg == "inspect only"));

        let codex = AgentCommandSpec::for_request(&request(
            AgentProvider::Codex,
            AgentAccessMode::WorkspaceWrite,
            "fix it",
        ))
        .unwrap();
        assert!(codex
            .args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"]));

        let deepseek = AgentCommandSpec::for_request(&request(
            AgentProvider::DeepSeek,
            AgentAccessMode::ReadOnly,
            "review",
        ))
        .unwrap();
        assert!(deepseek
            .args
            .windows(2)
            .any(|pair| pair == ["--provider", "deepseek"]));
        assert!(deepseek
            .args
            .windows(2)
            .any(|pair| pair == ["--tools", "read"]));
        assert!(deepseek
            .args
            .iter()
            .any(|arg| arg == "--disable-base-tools"));
        let jcode = AgentCommandSpec::for_request(&request(
            AgentProvider::Jcode,
            AgentAccessMode::ReadOnly,
            "review",
        ))
        .unwrap();
        assert!(!jcode.args.iter().any(|arg| arg == "--provider"));

        let oversized = request(
            AgentProvider::Claude,
            AgentAccessMode::ReadOnly,
            &"x".repeat(MAX_AGENT_PROMPT_BYTES + 1),
        );
        assert!(oversized.validate().is_err());

        for provider in [
            AgentProvider::Claude,
            AgentProvider::Codex,
            AgentProvider::Jcode,
            AgentProvider::DeepSeek,
        ] {
            let resumed = request(provider, AgentAccessMode::ReadOnly, "continue")
                .with_resume_session(Some("session-123".to_string()));
            let resumed = AgentCommandSpec::for_request(&resumed).unwrap();
            assert!(resumed.args.iter().any(|arg| arg == "session-123"));
            assert!(
                resumed.args.iter().any(|arg| arg == "--resume")
                    || provider == AgentProvider::Codex
            );
            if provider == AgentProvider::Codex {
                assert!(resumed
                    .args
                    .iter()
                    .any(|arg| arg == "sandbox_mode=\"read-only\""));
                assert!(resumed
                    .args
                    .iter()
                    .any(|arg| arg == "approval_policy=\"never\""));
            }
        }
    }
    // SDTEST-1674 — SDUC-475
    #[test]
    fn sdtest_1674_remote_command_quotes_workspace_and_encodes_stdin() {
        let mut run = request(
            AgentProvider::Codex,
            AgentAccessMode::ReadOnly,
            "line one\n$(touch /tmp/nope) ' quoted",
        );
        run.target = AgentTarget::Ssh {
            connection_id: Uuid::nil(),
            connection_label: "test-host".to_string(),
        };
        run.workdir = "/srv/customer's app".to_string();
        let command = AgentCommandSpec::for_request(&run)
            .unwrap()
            .remote_shell_command();
        assert!(command.contains("cd -- '/srv/customer'\\''s app' &&"));
        assert!(command.contains("$HOME/.local/bin"));
        assert!(command.contains("base64 -d"));
        assert!(!command.contains("touch /tmp/nope"));
    }

    // SDTEST-1675 — SDUC-475
    #[test]
    fn sdtest_1675_provider_streams_normalize_visible_text_and_errors() {
        assert_eq!(
            parse_stream_line(
                AgentProvider::Claude,
                r#"{"type":"system","subtype":"init","session_id":"claude-42"}"#,
            ),
            vec![
                AgentStreamEvent::Session("claude-42".to_string()),
                AgentStreamEvent::Ready,
            ]
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::Claude,
                r#"{"type":"system","subtype":"hook_started"}"#,
            ),
            Vec::<AgentStreamEvent>::new()
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::Claude,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}"#,
            ),
            vec![AgentStreamEvent::Text("done".to_string())]
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::Claude,
                r#"{"type":"assistant","isApiErrorMessage":true,"message":{"content":[{"type":"text","text":"API Error: blocked"}]}}"#,
            ),
            vec![AgentStreamEvent::Error("API Error: blocked".to_string())]
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::Codex,
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"fixed"}}"#,
            ),
            vec![AgentStreamEvent::Text("fixed".to_string())]
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::DeepSeek,
                r#"{"type":"error","message":"denied","is_error":true}"#,
            ),
            vec![AgentStreamEvent::Error("denied".to_string())]
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::DeepSeek,
                r#"{"type":"content.delta","session_id":"jcode-42","delta":"stream "}"#,
            ),
            vec![
                AgentStreamEvent::Session("jcode-42".to_string()),
                AgentStreamEvent::TextDelta("stream ".to_string()),
            ]
        );
        assert_eq!(
            parse_stream_line(
                AgentProvider::Codex,
                r#"{"type":"thread.started","thread_id":"codex-42"}"#,
            ),
            vec![
                AgentStreamEvent::Session("codex-42".to_string()),
                AgentStreamEvent::Ready,
            ]
        );
    }

    // SDTEST-1680 — SDUC-475
    #[cfg(unix)]
    #[test]
    fn sdtest_1680_remote_shell_transport_preserves_hostile_multiline_stdin() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let root = std::env::temp_dir().join(format!("shelldeck-agent-{}", Uuid::new_v4()));
        let workdir = root.join("customer's app");
        fs::create_dir_all(&workdir).unwrap();
        let capture = root.join("stdin.txt");
        let marker = root.join("must-not-exist");
        let fake = root.join("fake-agent");
        fs::write(
            &fake,
            format!(
                "#!/bin/sh\ncat > {}\n",
                shell_quote(&capture.to_string_lossy())
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&fake, permissions).unwrap();
        let prompt = format!("first line\n$(touch {})\n'quoted'", marker.display());
        let spec = AgentCommandSpec {
            program: fake.display().to_string(),
            args: vec!["--literal=$(false)".to_string()],
            cwd: workdir,
            stdin: Some(prompt.as_bytes().to_vec()),
            remove_env: Vec::new(),
        };

        let status = Command::new("sh")
            .arg("-c")
            .arg(spec.remote_shell_command())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read_to_string(&capture).unwrap(), prompt);
        assert!(!marker.exists(), "prompt text became remote shell syntax");
        fs::remove_dir_all(&root).unwrap();
    }

    // SDTEST-1771 — SDUC-491
    #[cfg(windows)]
    #[test]
    fn sdtest_1771_windows_job_object_terminates_owned_process_tree() {
        let mut command = Command::new("cmd.exe");
        command.args(["/C", "start \"\" /B /WAIT ping.exe -n 30 127.0.0.1 >NUL"]);
        configure_local_process(&mut command);
        let mut child = command.spawn().unwrap();
        let mut process_tree = LocalProcessTree::capture(child.id()).unwrap();
        let started = std::time::Instant::now();
        let status = terminate_local_process(&mut child, &mut process_tree).unwrap();
        assert!(!status.success());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
