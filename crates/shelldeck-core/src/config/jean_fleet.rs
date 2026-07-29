//! Jean fleet runtime — ShellDeck as a host for tenant/site-aware Jean
//! instances. Reads the fleet, registers this machine as a `runtime="shelldeck"`
//! instance, heartbeats + claims pending jobs, and (when authorized) executes
//! them by driving a headless coding agent (Jcode by default, legacy Claude Code
//! as an explicit rollout/fallback option).
//!
//! Endpoint: `{base}/api/manage/shelldeck/fleet` (Bearer device token).
//!
//! ## Safety
//! Executing a claimed job runs a local coding agent with file/edit/command powers in the
//! instance workdir. [`runtime_tick`] only auto-executes when `autonomy == "auto"`;
//! `"confirm"` returns the claimed job for an explicit human approval in the UI.
//! Execution goes through the [`JobExecutor`] trait so the loop is unit-tested
//! with a fake executor and the real `jcode run` / `claude -p` invocation only
//! runs live.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

/// Deserialize a string the server may send as JSON `null` → `""`.
fn de_nullable_string<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// Fleet timestamps come back as ISO-8601 strings (`created_at`/`updated_at`/
/// `last_seen_at`), sometimes numbers, sometimes null → epoch ms (`0.0` when
/// absent/unparseable).
fn de_flex_millis<'de, D>(d: D) -> std::result::Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flex {
        Num(f64),
        Str(String),
    }
    Ok(match Option::<Flex>::deserialize(d)? {
        Some(Flex::Num(n)) => n,
        Some(Flex::Str(s)) => {
            if let Ok(n) = s.parse::<f64>() {
                n
            } else {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.timestamp_millis() as f64)
                    .unwrap_or(0.0)
            }
        }
        None => 0.0,
    })
}

fn default_true() -> bool {
    true
}

fn default_timeout_seconds() -> u64 {
    30 * 60
}

/// Explicit rollout switch for the local fleet executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JeanRuntimeExecutorRollout {
    /// Preferred runtime: `jcode run`.
    Jcode,
    /// Legacy runtime: `claude -p`.
    Claude,
}

impl Default for JeanRuntimeExecutorRollout {
    fn default() -> Self {
        Self::Jcode
    }
}

/// Jcode can emit either a final JSON object or newline-delimited stream events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JcodeOutputFormat {
    Ndjson,
    Json,
}

impl Default for JcodeOutputFormat {
    fn default() -> Self {
        Self::Ndjson
    }
}

/// Transport preference for the Jcode executor.
///
/// `process` is the stable contract and remains the default. `acp`/`auto` are
/// ACP-ready configuration hooks: they perform a capability probe and strictly
/// fall back to the existing `jcode run` process transport until Jcode ACP has a
/// versioned public client contract we can safely execute against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JcodeTransportPreference {
    Process,
    Acp,
    Auto,
}

impl Default for JcodeTransportPreference {
    fn default() -> Self {
        Self::Process
    }
}

/// `[jean_runtime.executor]` — local agent command + rollout policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JeanRuntimeExecutorConfig {
    /// Explicit rollout setting. `jcode` uses `jcode run`; `claude` keeps the
    /// previous Claude Code behavior.
    #[serde(default)]
    pub rollout: JeanRuntimeExecutorRollout,
    /// Jcode binary path/name. Defaults to `$JCODE_BIN` or `jcode`.
    #[serde(default)]
    pub binary: Option<String>,
    /// Jcode provider passed as `--provider`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Preferred model. Overrides the fleet instance model when set.
    #[serde(default)]
    pub model: Option<String>,
    /// Jcode tool profile passed as `--tool-profile` (`full`, `minimal`, `none`, …).
    #[serde(default)]
    pub tool_profile: Option<String>,
    /// Jcode output mode. Defaults to streaming NDJSON.
    #[serde(default)]
    pub output_format: JcodeOutputFormat,
    /// Preferred Jcode transport. Defaults to the stable `jcode run` process
    /// path. `acp`/`auto` are feature-gated probe-only fallbacks today.
    #[serde(default)]
    pub transport: JcodeTransportPreference,
    /// Per-job hard timeout. On expiry the child process is killed. Defaults to 30m.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Keep legacy Claude as a startup fallback when Jcode cannot be launched.
    /// Fallback is intentionally not used after Jcode has started and returned a
    /// job error, avoiding duplicate edits/commands.
    #[serde(default = "default_true")]
    pub fallback_to_claude: bool,
    /// Optional Claude binary override for legacy mode/fallback.
    #[serde(default)]
    pub claude_binary: Option<String>,
    /// Optional Claude permission mode override. Defaults to `acceptEdits`.
    #[serde(default)]
    pub claude_permission_mode: Option<String>,
}

impl Default for JeanRuntimeExecutorConfig {
    fn default() -> Self {
        Self {
            rollout: JeanRuntimeExecutorRollout::Jcode,
            binary: None,
            provider: None,
            model: None,
            tool_profile: None,
            output_format: JcodeOutputFormat::Ndjson,
            transport: JcodeTransportPreference::Process,
            timeout_seconds: default_timeout_seconds(),
            fallback_to_claude: true,
            claude_binary: None,
            claude_permission_mode: None,
        }
    }
}

impl JeanRuntimeExecutorConfig {
    fn configured_model<'a>(&'a self, fleet_model: &'a str) -> &'a str {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fleet_model)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds.max(1))
    }

    pub fn self_report_label(&self) -> String {
        match self.rollout {
            JeanRuntimeExecutorRollout::Jcode => {
                let model = self.model.as_deref().unwrap_or("auto");
                let provider = self.provider.as_deref().unwrap_or("auto");
                format!("jcode/{provider}/{model}")
            }
            JeanRuntimeExecutorRollout::Claude => "claude legacy".to_string(),
        }
    }
}

/// Persisted `[jean_runtime]` config — whether this machine hosts a Jean
/// runtime, and its identity across restarts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JeanRuntimeConfig {
    /// Master switch. **Default `false`** — enabling this lets ShellDeck run
    /// Claude Code jobs on this machine.
    #[serde(default)]
    pub enabled: bool,
    /// Instance id returned by the first `register`, persisted so the same
    /// machine keeps its identity across restarts.
    #[serde(default)]
    pub instance_id: Option<String>,
    /// Working directory Claude Code runs in (defaults handled at register time).
    #[serde(default)]
    pub workdir: Option<String>,
    /// Instance display name (defaults to the machine hostname).
    #[serde(default)]
    pub name: Option<String>,
    /// Executor rollout + command configuration.
    #[serde(default)]
    pub executor: JeanRuntimeExecutorConfig,
}

impl JeanRuntimeConfig {
    pub fn job_timeout(&self) -> Duration {
        self.executor.timeout()
    }

    pub fn job_model(&self, fleet_model: &str) -> String {
        self.executor.configured_model(fleet_model).to_string()
    }

    pub fn job_executor(&self) -> ConfiguredJobExecutor {
        ConfiguredJobExecutor::from_config(&self.executor)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FleetEndpoint {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub url: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub user: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub pass: String,
}

/// Field names are snake_case per the fleet contract (unlike the camelCase
/// support/jeanclaude APIs), so no `rename_all` here.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct JeanInstance {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub tenant_id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub tenant_name: String,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub site_label: Option<String>,
    /// "server" | "shelldeck".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub runtime: String,
    #[serde(default)]
    pub endpoint: Option<FleetEndpoint>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub slack_channel: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub workdir: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub model: String,
    /// "confirm" | "auto".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub autonomy: String,
    #[serde(default)]
    pub enabled: bool,
    /// "online" | "busy" | "offline" | "unknown".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status_detail: String,
    /// Epoch ms; `0.0` = never seen (server may send ISO string / number / null).
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub last_seen_at: f64,
    #[serde(default)]
    pub agent_version: Option<String>,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub created_at: f64,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub updated_at: f64,
}

impl JeanInstance {
    pub fn is_shelldeck(&self) -> bool {
        self.runtime == "shelldeck"
    }
    pub fn is_auto(&self) -> bool {
        self.autonomy == "auto"
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct JeanJob {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub instance_id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub tenant_id: String,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub prompt: String,
    /// "manage" | "support:<id>" | "user" | "shelldeck" | "slack".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub source: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub requested_by: String,
    /// "pending" | "claimed" | "running" | "done" | "failed" | "cancelled".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub created_at: f64,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub updated_at: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FleetStats {
    #[serde(default)]
    pub online: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub pending: u32,
    #[serde(default)]
    pub running: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FleetSnapshot {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub instances: Vec<JeanInstance>,
    #[serde(default)]
    pub jobs: Vec<JeanJob>,
    #[serde(default)]
    pub stats: FleetStats,
}

/// Fields to register/update this machine as a runtime instance.
#[derive(Debug, Clone, Default)]
pub struct RegisterInstance {
    pub id: Option<String>,
    pub name: String,
    pub tenant_id: String,
    pub tenant_name: String,
    pub site_id: Option<String>,
    pub slack_channel: Option<String>,
    pub workdir: String,
    pub model: Option<String>,
    pub autonomy: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct InstanceResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    instance: JeanInstance,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct JobResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    job: Option<JeanJob>,
    #[serde(default)]
    error: Option<String>,
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

fn fleet_url(base_url: &str) -> String {
    format!(
        "{}/api/manage/shelldeck/fleet",
        base_url.trim_end_matches('/')
    )
}

fn check_status(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 => Err(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        )),
        s => Err(ShellDeckError::Connection(format!(
            "fleet request failed: HTTP {}",
            s
        ))),
    }
}

/// GET the tenant/site-filtered fleet snapshot.
pub fn get_fleet(base_url: &str, token: &str) -> Result<FleetSnapshot> {
    let client = http_client()?;
    let resp = client
        .get(fleet_url(base_url))
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("fleet request failed: {}", e)))?;
    check_status(resp.status().as_u16())?;
    resp.json::<FleetSnapshot>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid fleet payload: {}", e)))
}

fn post_json(
    base_url: &str,
    token: &str,
    body: serde_json::Value,
) -> Result<reqwest::blocking::Response> {
    let client = http_client()?;
    let resp = client
        .post(fleet_url(base_url))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("fleet request failed: {}", e)))?;
    check_status(resp.status().as_u16())?;
    Ok(resp)
}

fn instance_from(resp: reqwest::blocking::Response) -> Result<JeanInstance> {
    let parsed: InstanceResponse = resp
        .json()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid instance response: {}", e)))?;
    if parsed.ok {
        Ok(parsed.instance)
    } else {
        Err(ShellDeckError::Connection(
            parsed
                .error
                .unwrap_or_else(|| "fleet action refused".to_string()),
        ))
    }
}

fn job_from(resp: reqwest::blocking::Response) -> Result<Option<JeanJob>> {
    let parsed: JobResponse = resp
        .json()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid job response: {}", e)))?;
    if parsed.ok {
        Ok(parsed.job)
    } else {
        Err(ShellDeckError::Connection(
            parsed
                .error
                .unwrap_or_else(|| "fleet action refused".to_string()),
        ))
    }
}

/// Register (or, with `reg.id`, update) this machine as a runtime instance.
/// The server forces `runtime = "shelldeck"`.
pub fn register(base_url: &str, token: &str, reg: &RegisterInstance) -> Result<JeanInstance> {
    let mut instance = serde_json::json!({
        "name": reg.name,
        "tenant_id": reg.tenant_id,
        "tenant_name": reg.tenant_name,
        "workdir": reg.workdir,
    });
    let obj = instance.as_object_mut().unwrap();
    if let Some(id) = &reg.id {
        obj.insert("id".into(), serde_json::json!(id));
    }
    if let Some(s) = &reg.site_id {
        obj.insert("site_id".into(), serde_json::json!(s));
    }
    if let Some(c) = &reg.slack_channel {
        obj.insert("slack_channel".into(), serde_json::json!(c));
    }
    if let Some(m) = &reg.model {
        obj.insert("model".into(), serde_json::json!(m));
    }
    if let Some(a) = &reg.autonomy {
        obj.insert("autonomy".into(), serde_json::json!(a));
    }
    let resp = post_json(
        base_url,
        token,
        serde_json::json!({ "action": "register", "instance": instance }),
    )?;
    instance_from(resp)
}

/// Heartbeat this instance's liveness.
pub fn heartbeat(
    base_url: &str,
    token: &str,
    id: &str,
    status: &str,
    detail: Option<&str>,
    version: Option<&str>,
) -> Result<JeanInstance> {
    let mut body = serde_json::json!({ "action": "heartbeat", "id": id, "status": status });
    let obj = body.as_object_mut().unwrap();
    if let Some(d) = detail {
        obj.insert("detail".into(), serde_json::json!(d));
    }
    if let Some(v) = version {
        obj.insert("version".into(), serde_json::json!(v));
    }
    instance_from(post_json(base_url, token, body)?)
}

/// Claim the oldest pending job for this instance (or `None`).
pub fn claim(base_url: &str, token: &str, id: &str) -> Result<Option<JeanJob>> {
    job_from(post_json(
        base_url,
        token,
        serde_json::json!({ "action": "claim", "id": id }),
    )?)
}

/// Update a job's status (+ optional result).
pub fn update_job(
    base_url: &str,
    token: &str,
    job_id: &str,
    status: &str,
    result: Option<&str>,
) -> Result<Option<JeanJob>> {
    let mut body = serde_json::json!({ "action": "job", "jobId": job_id, "status": status });
    if let Some(r) = result {
        body.as_object_mut()
            .unwrap()
            .insert("result".into(), serde_json::json!(r));
    }
    job_from(post_json(base_url, token, body)?)
}

/// File a ticket to any instance.
pub fn dispatch(
    base_url: &str,
    token: &str,
    id: &str,
    prompt: &str,
    source: Option<&str>,
) -> Result<Option<JeanJob>> {
    let mut body = serde_json::json!({ "action": "dispatch", "id": id, "prompt": prompt });
    if let Some(s) = source {
        body.as_object_mut()
            .unwrap()
            .insert("source".into(), serde_json::json!(s));
    }
    job_from(post_json(base_url, token, body)?)
}

// ── execution ──────────────────────────────────────────────────────────────

/// Result of running one job's prompt.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub result: String,
    pub is_error: bool,
    /// Safe to retry with the legacy Claude fallback because the primary agent
    /// did not start user work (for example, binary not found). Runtime failures
    /// after a child launches are never fallbackable to avoid duplicate edits.
    pub fallback_allowed: bool,
}

impl JobOutcome {
    fn ok(result: impl Into<String>) -> Self {
        Self {
            result: result.into(),
            is_error: false,
            fallback_allowed: false,
        }
    }

    fn error(result: impl Into<String>) -> Self {
        Self {
            result: result.into(),
            is_error: true,
            fallback_allowed: false,
        }
    }

    fn fallbackable_error(result: impl Into<String>) -> Self {
        Self {
            result: result.into(),
            is_error: true,
            fallback_allowed: true,
        }
    }
}

/// Executes a job's prompt. Real impls drive headless Jcode/Claude Code; tests
/// use fakes. `Send + Sync` so the runtime loop can run on a background thread.
pub trait JobExecutor: Send + Sync {
    fn execute(&self, prompt: &str, workdir: &str, model: &str, timeout: Duration) -> JobOutcome;
}

fn opt_trimmed(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn validate_workdir(workdir: &str) -> std::result::Result<PathBuf, String> {
    let trimmed = workdir.trim();
    if trimmed.is_empty() {
        return Err("répertoire de travail vide".to_string());
    }
    let path = std::path::Path::new(trimmed);
    if !path.is_absolute() {
        return Err(format!("répertoire de travail non absolu: {}", trimmed));
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("répertoire de travail inaccessible ({}): {}", trimmed, e))?;
    if !canonical.is_dir() {
        return Err(format!(
            "répertoire de travail invalide (pas un dossier): {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

#[derive(Debug)]
struct ProcessOutput {
    stdout: String,
    stderr: String,
    status: Option<ExitStatus>,
    killed: bool,
}

fn read_pipe(pipe: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    })
}

fn wait_with_timeout(mut child: std::process::Child, timeout: Duration) -> ProcessOutput {
    let stdout_reader = read_pipe(child.stdout.take());
    let stderr_reader = read_pipe(child.stderr.take());
    let deadline = std::time::Instant::now() + timeout;
    let mut killed = false;
    let mut status = None;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    killed = true;
                    status = child.wait().ok();
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(_) => break,
        }
    }
    ProcessOutput {
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
        status,
        killed,
    }
}

/// Configured executor wrapper. Jcode is the normal rollout path; Claude remains
/// available as explicit legacy mode and as a startup-only fallback.
pub struct ConfiguredJobExecutor {
    primary: ExecutorImpl,
    fallback: Option<ClaudeExecutor>,
}

enum ExecutorImpl {
    Jcode(JcodeExecutor),
    Claude(ClaudeExecutor),
}

impl ConfiguredJobExecutor {
    pub fn from_config(config: &JeanRuntimeExecutorConfig) -> Self {
        let claude = ClaudeExecutor::from_config(config);
        match config.rollout {
            JeanRuntimeExecutorRollout::Jcode => Self {
                primary: ExecutorImpl::Jcode(JcodeExecutor::from_config(config)),
                fallback: config.fallback_to_claude.then_some(claude),
            },
            JeanRuntimeExecutorRollout::Claude => Self {
                primary: ExecutorImpl::Claude(claude),
                fallback: None,
            },
        }
    }
}

impl JobExecutor for ConfiguredJobExecutor {
    fn execute(&self, prompt: &str, workdir: &str, model: &str, timeout: Duration) -> JobOutcome {
        let primary = match &self.primary {
            ExecutorImpl::Jcode(exec) => exec.execute(prompt, workdir, model, timeout),
            ExecutorImpl::Claude(exec) => exec.execute(prompt, workdir, model, timeout),
        };
        if primary.is_error && primary.fallback_allowed {
            if let Some(fallback) = &self.fallback {
                let mut legacy = fallback.execute(prompt, workdir, model, timeout);
                if legacy.is_error {
                    legacy.result = format!(
                        "Jcode indisponible, puis fallback Claude en échec.\nJcode: {}\nClaude: {}",
                        primary.result, legacy.result
                    );
                }
                return legacy;
            }
        }
        primary
    }
}

/// Jcode executor — `jcode run --ndjson|--json --quiet --no-update --no-selfdev
/// -C <workdir> [--provider …] [--model …] [--tool-profile …] <prompt>`.
pub struct JcodeExecutor {
    pub bin: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tool_profile: Option<String>,
    pub output_format: JcodeOutputFormat,
    pub transport: JcodeTransportPreference,
}

impl JcodeExecutor {
    pub fn from_config(config: &JeanRuntimeExecutorConfig) -> Self {
        Self {
            bin: opt_trimmed(&config.binary)
                .map(str::to_string)
                .or_else(|| std::env::var("JCODE_BIN").ok())
                .unwrap_or_else(|| "jcode".to_string()),
            provider: opt_trimmed(&config.provider).map(str::to_string),
            model: opt_trimmed(&config.model).map(str::to_string),
            tool_profile: opt_trimmed(&config.tool_profile).map(str::to_string),
            output_format: config.output_format,
            transport: config.transport,
        }
    }
}

impl JobExecutor for JcodeExecutor {
    fn execute(&self, prompt: &str, workdir: &str, model: &str, timeout: Duration) -> JobOutcome {
        let workdir = match validate_workdir(workdir) {
            Ok(path) => path,
            Err(e) => return JobOutcome::error(format!("Workdir Jean invalide: {}", e)),
        };

        let process = ProcessJcodeTransport { executor: self };
        match self.transport {
            JcodeTransportPreference::Process => process.execute(prompt, &workdir, model, timeout),
            JcodeTransportPreference::Acp | JcodeTransportPreference::Auto => {
                let acp = AcpJcodeTransport { executor: self };
                let probe = acp.probe(Duration::from_secs(2).min(timeout));
                if probe.available {
                    let outcome = acp.execute(prompt, &workdir, model, timeout);
                    if !outcome.is_error || !outcome.fallback_allowed {
                        return outcome;
                    }
                }

                let mut fallback = process.execute(prompt, &workdir, model, timeout);
                if fallback.is_error {
                    fallback.result = format!(
                        "Transport Jcode ACP indisponible, puis fallback process en échec.\nACP: {}\nProcess: {}",
                        probe.reason, fallback.result
                    );
                }
                fallback
            }
        }
    }
}

struct ProcessJcodeTransport<'a> {
    executor: &'a JcodeExecutor,
}

trait JcodeTransport {
    fn probe(&self, timeout: Duration) -> JcodeTransportProbe;
    fn execute(
        &self,
        prompt: &str,
        workdir: &std::path::Path,
        model: &str,
        timeout: Duration,
    ) -> JobOutcome;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JcodeTransportProbe {
    pub available: bool,
    pub reason: String,
}

impl JcodeTransportProbe {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: reason.into(),
        }
    }
}

impl JcodeTransport for ProcessJcodeTransport<'_> {
    fn probe(&self, _timeout: Duration) -> JcodeTransportProbe {
        JcodeTransportProbe {
            available: true,
            reason: "stable jcode run process transport".to_string(),
        }
    }

    fn execute(
        &self,
        prompt: &str,
        workdir: &std::path::Path,
        model: &str,
        timeout: Duration,
    ) -> JobOutcome {
        use std::process::{Command, Stdio};

        let mut args: Vec<String> = vec![
            "run".into(),
            match self.executor.output_format {
                JcodeOutputFormat::Ndjson => "--ndjson".into(),
                JcodeOutputFormat::Json => "--json".into(),
            },
            "--quiet".into(),
            "--no-update".into(),
            "--no-selfdev".into(),
            "-C".into(),
            workdir.display().to_string(),
        ];
        if let Some(provider) = &self.executor.provider {
            args.push("--provider".into());
            args.push(provider.clone());
        }
        let effective_model = self
            .executor
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(model.trim());
        if !effective_model.is_empty() {
            args.push("--model".into());
            args.push(effective_model.to_string());
        }
        if let Some(profile) = &self.executor.tool_profile {
            args.push("--tool-profile".into());
            args.push(profile.clone());
        }
        args.push(prompt.to_string());

        let mut cmd = Command::new(&self.executor.bin);
        cmd.args(&args)
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Linux can briefly return ETXTBSY while a freshly replaced executable is still being
        // closed by its writer. Retry only that transient condition; every other launch failure
        // remains immediately fallbackable and the bounded retries do not affect cancellation.
        let mut attempts = 0;
        let child = loop {
            match cmd.spawn() {
                Ok(c) => break c,
                Err(e) if e.raw_os_error() == Some(26) && attempts < 3 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    return JobOutcome::fallbackable_error(format!(
                        "Impossible de lancer Jcode ({}): {}",
                        self.executor.bin, e
                    ));
                }
            }
        };

        let output = wait_with_timeout(child, timeout);
        parse_jcode_output(&output, self.executor.output_format, timeout)
    }
}

struct AcpJcodeTransport<'a> {
    executor: &'a JcodeExecutor,
}

impl JcodeTransport for AcpJcodeTransport<'_> {
    fn probe(&self, timeout: Duration) -> JcodeTransportProbe {
        probe_jcode_acp(&self.executor.bin, timeout)
    }

    fn execute(
        &self,
        _prompt: &str,
        _workdir: &std::path::Path,
        _model: &str,
        _timeout: Duration,
    ) -> JobOutcome {
        JobOutcome::fallbackable_error(
            "Transport Jcode ACP désactivé: aucun contrat client ACP versionné et public n'est encore validé pour ShellDeck.",
        )
    }
}

#[cfg(not(feature = "jcode-acp"))]
fn probe_jcode_acp(_bin: &str, _timeout: Duration) -> JcodeTransportProbe {
    JcodeTransportProbe::unavailable(
        "support ACP Jcode non compilé (feature shelldeck-core/jcode-acp absente); fallback process utilisé",
    )
}

#[cfg(feature = "jcode-acp")]
fn probe_jcode_acp(bin: &str, timeout: Duration) -> JcodeTransportProbe {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(bin);
    cmd.arg("acp")
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return JcodeTransportProbe::unavailable(format!(
                "commande ACP Jcode indisponible ({} acp --help): {}; fallback process utilisé",
                bin, err
            ));
        }
    };
    let output = wait_with_timeout(child, timeout.max(Duration::from_secs(1)));
    if output.killed {
        return JcodeTransportProbe::unavailable(
            "probe ACP Jcode expiré; fallback process utilisé".to_string(),
        );
    }
    let help = format!("{}\n{}", output.stdout, output.stderr);
    if !status_success(output.status)
        || !help.contains("Agent Client Protocol")
        || !help.contains("acp")
    {
        return JcodeTransportProbe::unavailable(
            "la CLI Jcode courante n'annonce pas un adaptateur ACP compatible; fallback process utilisé",
        );
    }
    JcodeTransportProbe::unavailable(
        "adaptateur ACP Jcode détecté, mais exécution désactivée: ShellDeck n'a trouvé qu'un contrat de source CLI non versionné, pas une API client publique stable; fallback process utilisé",
    )
}

/// Real executor — mirrors the bot's `claude.ts`: `claude -p --output-format
/// stream-json --verbose --permission-mode acceptEdits [--model …]`, prompt on
/// stdin, subscription auth (drops `ANTHROPIC_API_KEY`, keeps
/// `CLAUDE_CODE_OAUTH_TOKEN`), cwd = workdir, killed after `timeout`.
pub struct ClaudeExecutor {
    pub bin: String,
    pub permission_mode: String,
}

impl Default for ClaudeExecutor {
    fn default() -> Self {
        Self {
            bin: std::env::var("CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string()),
            permission_mode: "acceptEdits".to_string(),
        }
    }
}

impl ClaudeExecutor {
    pub fn from_config(config: &JeanRuntimeExecutorConfig) -> Self {
        Self {
            bin: opt_trimmed(&config.claude_binary)
                .map(str::to_string)
                .or_else(|| std::env::var("CLAUDE_BIN").ok())
                .unwrap_or_else(|| "claude".to_string()),
            permission_mode: opt_trimmed(&config.claude_permission_mode)
                .unwrap_or("acceptEdits")
                .to_string(),
        }
    }
}

impl JobExecutor for ClaudeExecutor {
    fn execute(&self, prompt: &str, workdir: &str, model: &str, timeout: Duration) -> JobOutcome {
        use std::process::{Command, Stdio};

        let workdir = match validate_workdir(workdir) {
            Ok(path) => path,
            Err(e) => return JobOutcome::error(format!("Workdir Jean invalide: {}", e)),
        };

        let mut args: Vec<String> = vec![
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--permission-mode".into(),
            self.permission_mode.clone(),
        ];
        if !model.trim().is_empty() {
            args.push("--model".into());
            args.push(model.to_string());
        }

        let mut cmd = Command::new(&self.bin);
        cmd.args(&args)
            .current_dir(&workdir)
            .env_remove("ANTHROPIC_API_KEY") // force subscription auth
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return JobOutcome::fallbackable_error(format!(
                    "Impossible de lancer Claude Code ({}): {}",
                    self.bin, e
                ));
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes());
            // dropping stdin closes it (EOF)
        }

        let output = wait_with_timeout(child, timeout);
        parse_claude_stream_json(&output, timeout)
    }
}

fn timeout_outcome(timeout: Duration) -> JobOutcome {
    JobOutcome::error(format!(
        "Délai dépassé ({}s) — exécution interrompue.",
        timeout.as_secs().max(1)
    ))
}

fn status_success(status: Option<ExitStatus>) -> bool {
    status.map(|s| s.success()).unwrap_or(true)
}

fn extract_string(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = value.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(text) = item
                .get("text")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("content").and_then(|v| v.as_str()))
            {
                parts.push(text.to_string());
            } else if let Some(text) = item.as_str() {
                parts.push(text.to_string());
            }
        }
        if !parts.is_empty() {
            return Some(parts.join(""));
        }
    }
    if let Some(obj) = value.as_object() {
        for key in [
            "result", "response", "message", "content", "text", "output", "final",
        ] {
            if let Some(text) = obj.get(key).and_then(extract_string) {
                return Some(text);
            }
        }
    }
    None
}

fn json_result(value: &serde_json::Value) -> Option<JobOutcome> {
    let text = [
        "result", "response", "message", "content", "text", "output", "final",
    ]
    .iter()
    .find_map(|key| value.get(*key).and_then(extract_string))
    .or_else(|| extract_string(value.get("delta")?));
    let text = text?;
    let is_error = value
        .get("is_error")
        .and_then(|e| e.as_bool())
        .or_else(|| value.get("error").and_then(|e| e.as_bool()))
        .unwrap_or(false)
        || value.get("status").and_then(|s| s.as_str()) == Some("error");
    Some(JobOutcome {
        result: text,
        is_error,
        fallback_allowed: false,
    })
}

fn fallback_result(agent: &str, stdout: &str, stderr: &str) -> JobOutcome {
    let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (true, true) => format!("{} n'a produit aucune sortie.", agent),
        (false, true) => stdout.trim().chars().take(2000).collect(),
        (true, false) => stderr.trim().chars().take(2000).collect(),
        (false, false) => format!(
            "{}\n\nstderr:\n{}",
            stdout.trim().chars().take(1400).collect::<String>(),
            stderr.trim().chars().take(600).collect::<String>()
        ),
    };
    JobOutcome::error(combined)
}

fn parse_jcode_output(
    output: &ProcessOutput,
    _format: JcodeOutputFormat,
    timeout: Duration,
) -> JobOutcome {
    if output.killed {
        return timeout_outcome(timeout);
    }
    let mut last_result = None;
    let mut stream_text = String::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(output.stdout.trim()) {
        last_result = json_result(&v);
    } else {
        for line in output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(text) = v.get("delta").and_then(extract_string) {
                    stream_text.push_str(&text);
                }
                if let Some(outcome) = json_result(&v) {
                    last_result = Some(outcome);
                }
            }
        }
    }
    let mut outcome = last_result.unwrap_or_else(|| {
        if !stream_text.trim().is_empty() {
            JobOutcome::ok(stream_text.trim().to_string())
        } else {
            fallback_result("Jcode", &output.stdout, &output.stderr)
        }
    });
    if !status_success(output.status) {
        outcome.is_error = true;
        if outcome.result.trim().is_empty() && !output.stderr.trim().is_empty() {
            outcome.result = output.stderr.trim().chars().take(2000).collect();
        }
    }
    outcome
}

fn parse_claude_stream_json(output: &ProcessOutput, timeout: Duration) -> JobOutcome {
    if output.killed {
        return timeout_outcome(timeout);
    }
    let mut last_result: Option<JobOutcome> = None;
    for line in output.stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("result") {
                let text = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                last_result = Some(JobOutcome {
                    result: text,
                    is_error,
                    fallback_allowed: false,
                });
            }
        }
    }
    let mut outcome = last_result
        .unwrap_or_else(|| fallback_result("Claude Code", &output.stdout, &output.stderr));
    if !status_success(output.status) {
        outcome.is_error = true;
    }
    outcome
}

/// Parse the final `result` event out of Claude Code's stream-json stdout.
#[cfg(test)]
fn parse_stream_json(stdout: &str, killed: bool) -> JobOutcome {
    let output = ProcessOutput {
        stdout: stdout.to_string(),
        stderr: String::new(),
        status: None,
        killed,
    };
    parse_claude_stream_json(&output, Duration::from_secs(default_timeout_seconds()))
}

/// Execute a claimed job end-to-end: mark `running`, run it, then `done`/`failed`
/// with a brief result. Used for `auto` instances and for a human-approved
/// `confirm` job.
pub fn execute_job(
    base_url: &str,
    token: &str,
    job: &JeanJob,
    workdir: &str,
    model: &str,
    exec: &dyn JobExecutor,
    timeout: Duration,
) -> Result<JeanJob> {
    let _ = update_job(base_url, token, &job.id, "running", None)?;
    let outcome = exec.execute(&job.prompt, workdir, model, timeout);
    let status = if outcome.is_error { "failed" } else { "done" };
    let brief: String = outcome.result.chars().take(4000).collect();
    let updated = update_job(base_url, token, &job.id, status, Some(&brief))?;
    Ok(updated.unwrap_or_else(|| JeanJob {
        id: job.id.clone(),
        status: status.to_string(),
        result: Some(brief),
        ..job.clone()
    }))
}

/// Outcome of one runtime tick.
#[derive(Debug, Clone, Default)]
pub struct TickResult {
    /// Set (only for `confirm` instances) when a job was claimed and awaits a
    /// human's explicit "Exécuter" in the UI.
    pub awaiting_confirm: Option<JeanJob>,
}

/// One runtime iteration: heartbeat, claim a pending job, and — for `auto`
/// instances — execute it. For `confirm`, the claimed job is returned so the UI
/// can surface it for approval (NOT executed here).
///
/// `busy` is a flag the caller flips so it never runs two jobs at once
/// (concurrency 1 per ShellDeck runtime).
#[allow(clippy::too_many_arguments)]
pub fn runtime_tick(
    base_url: &str,
    token: &str,
    instance_id: &str,
    workdir: &str,
    model: &str,
    autonomy: &str,
    version: &str,
    exec: &dyn JobExecutor,
    timeout: Duration,
) -> Result<TickResult> {
    heartbeat(base_url, token, instance_id, "online", None, Some(version))?;
    let Some(job) = claim(base_url, token, instance_id)? else {
        return Ok(TickResult::default());
    };

    if autonomy == "auto" {
        let _ = heartbeat(
            base_url,
            token,
            instance_id,
            "busy",
            Some("exécution"),
            Some(version),
        );
        let r = execute_job(base_url, token, &job, workdir, model, exec, timeout);
        let _ = heartbeat(base_url, token, instance_id, "online", None, Some(version));
        r?;
        Ok(TickResult::default())
    } else {
        Ok(TickResult {
            awaiting_confirm: Some(job),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    /// Fake executor — records the prompt, returns a canned outcome. The real
    /// `claude -p` is NEVER run in tests.
    struct FakeExecutor {
        outcome: JobOutcome,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl JobExecutor for FakeExecutor {
        fn execute(&self, prompt: &str, _workdir: &str, _model: &str, _t: Duration) -> JobOutcome {
            self.seen.lock().unwrap().push(prompt.to_string());
            self.outcome.clone()
        }
    }

    struct Mock {
        url: String,
        posts: Arc<Mutex<Vec<String>>>,
        _handle: std::thread::JoinHandle<()>,
    }

    /// A canned fleet mock: requires Bearer auth, records POST bodies, and serves
    /// register/heartbeat/claim/job/dispatch + GET fleet fixtures.
    fn start_mock(claim_returns_job: bool) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let posts = Arc::new(Mutex::new(Vec::<String>::new()));
        let posts2 = posts.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..64 {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let mut auth = String::new();
                let mut clen = 0usize;
                loop {
                    let mut l = String::new();
                    if reader.read_line(&mut l).unwrap_or(0) == 0 {
                        break;
                    }
                    let t = l.trim_end();
                    if t.is_empty() {
                        break;
                    }
                    if let Some(idx) = t.find(':') {
                        let k = t[..idx].trim().to_ascii_lowercase();
                        let v = t[idx + 1..].trim();
                        if k == "authorization" {
                            auth = v.to_string();
                        } else if k == "content-length" {
                            clen = v.parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = String::new();
                if clen > 0 {
                    let mut b = vec![0u8; clen];
                    let _ = reader.read_exact(&mut b);
                    body = String::from_utf8_lossy(&b).into_owned();
                }
                let method = request_line.split_whitespace().next().unwrap_or("");

                let (status, out): (u16, String) = if !auth.starts_with("Bearer ") {
                    (401, r#"{"ok":false,"error":"unauthorized"}"#.into())
                } else if method == "GET" {
                    (
                        200,
                        r#"{"ok":true,"instances":[
                            {"id":"i1","name":"activ-2","tenant_id":"t1","tenant_name":"Acme",
                             "runtime":"shelldeck","status":"online","autonomy":"auto","enabled":true,
                             "workdir":"/x","last_seen_at":1751470000000}
                          ],"jobs":[
                            {"id":"j1","instance_id":"i1","tenant_id":"t1","prompt":"corrige X",
                             "source":"manage","requested_by":"U1","status":"pending","result":null}
                          ],"stats":{"online":1,"total":1,"pending":1,"running":0}}"#
                            .into(),
                    )
                } else {
                    posts2.lock().unwrap().push(body.clone());
                    let action = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("action").and_then(|a| a.as_str()).map(String::from))
                        .unwrap_or_default();
                    match action.as_str() {
                        "register" | "heartbeat" => (
                            200,
                            r#"{"ok":true,"instance":{"id":"i1","name":"activ-2","runtime":"shelldeck","autonomy":"auto","status":"online"}}"#.into(),
                        ),
                        "claim" => {
                            if claim_returns_job {
                                (200, r#"{"ok":true,"job":{"id":"j1","instance_id":"i1","prompt":"corrige X","status":"claimed"}}"#.into())
                            } else {
                                (200, r#"{"ok":true,"job":null}"#.into())
                            }
                        }
                        "job" => (200, r#"{"ok":true,"job":{"id":"j1","status":"done"}}"#.into()),
                        "dispatch" => (200, r#"{"ok":true,"job":{"id":"j2","status":"pending"}}"#.into()),
                        _ => (200, r#"{"ok":true}"#.into()),
                    }
                };
                let resp = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    out.as_bytes().len(),
                    out
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        Mock {
            url: format!("http://127.0.0.1:{}", port),
            posts,
            _handle: handle,
        }
    }

    const TOKEN: &str = "sd_faketoken";

    fn temp_dir(name: &str) -> PathBuf {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("shelldeck-jean-fleet-{name}-{id}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(unix)]
    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    #[cfg(unix)]
    fn fake_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{}\n", body)).unwrap();
        let mut perm = fs::metadata(&path).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&path, perm).unwrap();
        path
    }

    #[cfg(windows)]
    fn fake_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(format!("{name}.cmd"));
        fs::write(&path, body).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn jcode_executor_uses_run_ndjson_flags_and_prompt_arg() {
        let dir = temp_dir("jcode-flags");
        let workdir = dir.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let args_file = dir.join("args.txt");
        let cwd_file = dir.join("cwd.txt");
        let stdin_file = dir.join("stdin.txt");
        let bin = fake_executable(
            &dir,
            "fake-jcode",
            &format!(
                r#"printf '%s\n' "$PWD" > {}
printf '%s\n' "$@" > {}
cat > {}
printf '%s\n' '{{"type":"result","result":"jcode done","is_error":false}}'
"#,
                shell_quote(&cwd_file),
                shell_quote(&args_file),
                shell_quote(&stdin_file)
            ),
        );

        let exec = JcodeExecutor::from_config(&JeanRuntimeExecutorConfig {
            binary: Some(bin.display().to_string()),
            provider: Some("openai-api".into()),
            model: Some("gpt-5.5".into()),
            tool_profile: Some("minimal".into()),
            output_format: JcodeOutputFormat::Ndjson,
            ..Default::default()
        });
        let outcome = exec.execute(
            "fix it",
            workdir.to_str().unwrap(),
            "ignored",
            Duration::from_secs(5),
        );

        assert!(!outcome.is_error, "{:?}", outcome);
        assert_eq!(outcome.result, "jcode done");
        let canonical = workdir.canonicalize().unwrap();
        let canonical = canonical.to_str().unwrap();
        assert_eq!(fs::read_to_string(&cwd_file).unwrap().trim(), canonical);
        assert_eq!(fs::read_to_string(&stdin_file).unwrap(), "");
        let args: Vec<String> = fs::read_to_string(&args_file)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--ndjson".to_string()));
        assert!(args.contains(&"--quiet".to_string()));
        assert!(args.contains(&"--no-update".to_string()));
        assert!(args.contains(&"--no-selfdev".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-C" && w[1] == canonical));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--provider" && w[1] == "openai-api")
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--model" && w[1] == "gpt-5.5")
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--tool-profile" && w[1] == "minimal")
        );
        assert_eq!(args.last().map(String::as_str), Some("fix it"));
    }

    #[cfg(unix)]
    #[test]
    fn jcode_executor_parses_json_output() {
        let dir = temp_dir("jcode-json");
        let workdir = dir.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let bin = fake_executable(
            &dir,
            "fake-jcode-json",
            r#"printf '%s\n' '{"response":"json done","is_error":false}'"#,
        );
        let exec = JcodeExecutor::from_config(&JeanRuntimeExecutorConfig {
            binary: Some(bin.display().to_string()),
            output_format: JcodeOutputFormat::Json,
            ..Default::default()
        });
        let outcome = exec.execute("fix", workdir.to_str().unwrap(), "", Duration::from_secs(5));
        assert_eq!(outcome.result, "json done");
        assert!(!outcome.is_error);
    }

    #[test]
    fn jcode_acp_probe_is_explicitly_disabled_by_contract() {
        let probe = probe_jcode_acp("missing-jcode-for-probe", Duration::from_secs(1));
        assert!(!probe.available);
        assert!(
            probe.reason.contains("fallback process")
                || probe.reason.contains("fallback process utilisé"),
            "{}",
            probe.reason
        );
    }

    #[cfg(unix)]
    #[test]
    fn jcode_acp_transport_falls_back_to_process_run() {
        let dir = temp_dir("jcode-acp-fallback");
        let workdir = dir.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let calls_file = dir.join("calls.txt");
        let bin = fake_executable(
            &dir,
            "fake-jcode-acp-fallback",
            &format!(
                r#"printf '%s\n' "$@" >> {}
if [ "${{1:-}}" = "acp" ]; then
  printf '%s\n' 'Run as an Agent Client Protocol (ACP) adapter backed by the Jcode daemon'
  exit 0
fi
printf '%s\n' '{{"type":"result","result":"process fallback","is_error":false}}'
"#,
                shell_quote(&calls_file),
            ),
        );

        let exec = JcodeExecutor::from_config(&JeanRuntimeExecutorConfig {
            binary: Some(bin.display().to_string()),
            transport: JcodeTransportPreference::Acp,
            ..Default::default()
        });
        let outcome = exec.execute(
            "fix via fallback",
            workdir.to_str().unwrap(),
            "",
            Duration::from_secs(5),
        );

        assert!(!outcome.is_error, "{:?}", outcome);
        assert_eq!(outcome.result, "process fallback");
        let calls = fs::read_to_string(calls_file).unwrap();
        assert!(calls.contains("run\n"), "{calls}");
        assert!(calls.contains("fix via fallback"), "{calls}");
    }

    #[cfg(unix)]
    #[test]
    fn jcode_executor_rejects_relative_or_missing_workdir_before_spawn() {
        let dir = temp_dir("jcode-workdir");
        let touched = dir.join("touched");
        let bin = fake_executable(
            &dir,
            "fake-jcode-workdir",
            &format!("touch {}", shell_quote(&touched)),
        );
        let exec = JcodeExecutor::from_config(&JeanRuntimeExecutorConfig {
            binary: Some(bin.display().to_string()),
            ..Default::default()
        });

        let relative = exec.execute("fix", "relative/path", "", Duration::from_secs(5));
        assert!(relative.is_error);
        assert!(
            relative.result.contains("non absolu"),
            "{}",
            relative.result
        );
        assert!(
            !touched.exists(),
            "executor must not spawn for invalid workdirs"
        );

        let missing = dir.join("missing");
        let missing = exec.execute("fix", missing.to_str().unwrap(), "", Duration::from_secs(5));
        assert!(missing.is_error);
        assert!(
            missing.result.contains("inaccessible"),
            "{}",
            missing.result
        );
        assert!(
            !touched.exists(),
            "executor must not spawn for missing workdirs"
        );
    }

    #[cfg(unix)]
    #[test]
    fn jcode_executor_kills_child_on_timeout() {
        let dir = temp_dir("jcode-timeout");
        let workdir = dir.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let bin = fake_executable(&dir, "fake-jcode-sleep", "sleep 5");
        let exec = JcodeExecutor::from_config(&JeanRuntimeExecutorConfig {
            binary: Some(bin.display().to_string()),
            ..Default::default()
        });
        let outcome = exec.execute("fix", workdir.to_str().unwrap(), "", Duration::from_secs(1));
        assert!(outcome.is_error);
        assert!(
            outcome.result.contains("Délai dépassé"),
            "{}",
            outcome.result
        );
    }

    #[cfg(unix)]
    #[test]
    fn jcode_acp_fallback_preserves_process_timeout_cancellation() {
        let dir = temp_dir("jcode-acp-timeout");
        let workdir = dir.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let bin = fake_executable(&dir, "fake-jcode-acp-timeout", "sleep 5");
        let exec = JcodeExecutor::from_config(&JeanRuntimeExecutorConfig {
            binary: Some(bin.display().to_string()),
            transport: JcodeTransportPreference::Acp,
            ..Default::default()
        });

        let outcome = exec.execute("fix", workdir.to_str().unwrap(), "", Duration::from_secs(1));
        assert!(outcome.is_error);
        assert!(
            outcome.result.contains("Délai dépassé"),
            "{}",
            outcome.result
        );
        assert!(
            outcome.result.contains("ACP") || outcome.result.contains("Jcode"),
            "{}",
            outcome.result
        );
    }

    #[cfg(unix)]
    #[test]
    fn configured_executor_falls_back_to_legacy_claude_only_when_jcode_cannot_start() {
        let dir = temp_dir("jcode-fallback");
        let workdir = dir.join("work");
        fs::create_dir_all(&workdir).unwrap();
        let stdin_file = dir.join("claude-stdin.txt");
        let claude = fake_executable(
            &dir,
            "fake-claude",
            &format!(
                r#"cat > {}
printf '%s\n' '{{"type":"result","result":"claude fallback","is_error":false}}'
"#,
                shell_quote(&stdin_file)
            ),
        );
        let exec = ConfiguredJobExecutor::from_config(&JeanRuntimeExecutorConfig {
            rollout: JeanRuntimeExecutorRollout::Jcode,
            binary: Some(dir.join("missing-jcode").display().to_string()),
            fallback_to_claude: true,
            claude_binary: Some(claude.display().to_string()),
            ..Default::default()
        });

        let outcome = exec.execute(
            "legacy please",
            workdir.to_str().unwrap(),
            "",
            Duration::from_secs(5),
        );
        assert!(!outcome.is_error, "{:?}", outcome);
        assert_eq!(outcome.result, "claude fallback");
        assert_eq!(fs::read_to_string(stdin_file).unwrap(), "legacy please");
    }

    #[test]
    fn get_fleet_parses() {
        let m = start_mock(false);
        let f = get_fleet(&m.url, TOKEN).expect("fleet");
        assert!(f.ok);
        assert_eq!(f.instances.len(), 1);
        assert!(f.instances[0].is_shelldeck());
        assert_eq!(f.stats.pending, 1);
        assert_eq!(f.jobs[0].status, "pending");
    }

    #[test]
    fn register_heartbeat_dispatch() {
        let m = start_mock(false);
        let reg = RegisterInstance {
            name: "activ-2".into(),
            tenant_id: "t1".into(),
            tenant_name: "Acme".into(),
            workdir: "/x".into(),
            autonomy: Some("confirm".into()),
            ..Default::default()
        };
        let inst = register(&m.url, TOKEN, &reg).expect("register");
        assert_eq!(inst.id, "i1");
        heartbeat(&m.url, TOKEN, "i1", "online", None, Some("0.3.1")).expect("hb");
        dispatch(&m.url, TOKEN, "i1", "fais X", Some("shelldeck")).expect("dispatch");

        let posts = m.posts.lock().unwrap();
        assert!(posts.iter().any(|b| b.contains("\"action\":\"register\"")));
        assert!(posts.iter().any(|b| b.contains("\"action\":\"heartbeat\"")));
        assert!(posts.iter().any(|b| b.contains("\"action\":\"dispatch\"")));
    }

    #[test]
    fn auto_tick_claims_and_executes() {
        let m = start_mock(true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let exec = FakeExecutor {
            outcome: JobOutcome {
                result: "fait".into(),
                is_error: false,
                fallback_allowed: false,
            },
            seen: seen.clone(),
        };
        let r = runtime_tick(
            &m.url,
            TOKEN,
            "i1",
            "/x",
            "",
            "auto",
            "0.3.1",
            &exec,
            Duration::from_secs(5),
        )
        .expect("tick");
        assert!(r.awaiting_confirm.is_none());
        // The fake executor ran on the claimed prompt.
        assert_eq!(seen.lock().unwrap().as_slice(), &["corrige X".to_string()]);
        // The loop posted heartbeat + claim + running + done.
        let posts = m.posts.lock().unwrap();
        assert!(posts.iter().any(|b| b.contains("\"status\":\"running\"")));
        assert!(posts.iter().any(|b| b.contains("\"status\":\"done\"")));
    }

    #[test]
    fn confirm_tick_claims_but_does_not_execute() {
        let m = start_mock(true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let exec = FakeExecutor {
            outcome: JobOutcome {
                result: "x".into(),
                is_error: false,
                fallback_allowed: false,
            },
            seen: seen.clone(),
        };
        let r = runtime_tick(
            &m.url,
            TOKEN,
            "i1",
            "/x",
            "",
            "confirm",
            "0.3.1",
            &exec,
            Duration::from_secs(5),
        )
        .expect("tick");
        let job = r.awaiting_confirm.expect("job awaiting confirm");
        assert_eq!(job.id, "j1");
        // Crucially, the executor was NOT called and no running/done was posted.
        assert!(seen.lock().unwrap().is_empty());
        let posts = m.posts.lock().unwrap();
        assert!(!posts.iter().any(|b| b.contains("\"status\":\"running\"")));
    }

    #[test]
    fn wrong_auth_surfaces_401() {
        let m = start_mock(false);
        let err = get_fleet(&m.url, "").unwrap_err();
        assert!(err.to_string().contains("401"), "got {}", err);
    }

    #[test]
    fn parses_iso_and_null_timestamps() {
        // Exactly the live register shape: ISO-string created_at/updated_at,
        // null last_seen_at, empty strings for optional fields.
        let json = r#"{
          "id":"4365eee9","name":"x","tenant_id":"t","tenant_name":"fghfg",
          "site_id":null,"site_label":null,"runtime":"shelldeck","endpoint":null,
          "slack_channel":"","workdir":"/tmp","model":"","autonomy":"confirm",
          "enabled":true,"status":"unknown","status_detail":"","last_seen_at":null,
          "agent_version":null,"created_at":"2026-07-02T20:54:11.843Z",
          "updated_at":"2026-07-02T20:54:11.843Z"
        }"#;
        let inst: JeanInstance = serde_json::from_str(json).expect("parse live register shape");
        assert_eq!(inst.id, "4365eee9");
        assert!(inst.is_shelldeck());
        assert_eq!(inst.autonomy, "confirm");
        assert!(inst.created_at > 0.0, "ISO created_at should parse to ms");
        assert_eq!(inst.last_seen_at, 0.0, "null last_seen_at → 0");
        assert!(inst.endpoint.is_none());
    }

    #[test]
    fn parse_stream_json_finds_result() {
        let out = "{\"type\":\"assistant\"}\n{\"type\":\"result\",\"result\":\"ok fini\",\"is_error\":false}\n";
        let o = parse_stream_json(out, false);
        assert_eq!(o.result, "ok fini");
        assert!(!o.is_error);
        // Timeout kill path.
        assert!(parse_stream_json("", true).is_error);
    }
}
