//! Jean fleet runtime — ShellDeck as a host for tenant/site-aware Jean
//! instances. Reads the fleet, registers this machine as a `runtime="shelldeck"`
//! instance, heartbeats + claims pending jobs, and (when authorized) executes
//! them by driving headless Claude Code.
//!
//! Endpoint: `{base}/api/manage/shelldeck/fleet` (Bearer device token).
//!
//! ## Safety
//! Executing a claimed job runs Claude Code with file/edit/command powers in the
//! instance workdir. [`runtime_tick`] only auto-executes when `autonomy == "auto"`;
//! `"confirm"` returns the claimed job for an explicit human approval in the UI.
//! Execution goes through the [`JobExecutor`] trait so the loop is unit-tested
//! with a fake executor and the real `claude -p` invocation only runs live.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::io::Write;
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
}

/// Executes a job's prompt. The real impl drives headless Claude Code; tests use
/// a fake. `Send + Sync` so the runtime loop can run it on a background thread.
pub trait JobExecutor: Send + Sync {
    fn execute(&self, prompt: &str, workdir: &str, model: &str, timeout: Duration) -> JobOutcome;
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

impl JobExecutor for ClaudeExecutor {
    fn execute(&self, prompt: &str, workdir: &str, model: &str, timeout: Duration) -> JobOutcome {
        use std::process::{Command, Stdio};

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
            .current_dir(workdir)
            .env_remove("ANTHROPIC_API_KEY") // force subscription auth
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return JobOutcome {
                    result: format!("Impossible de lancer Claude Code ({}): {}", self.bin, e),
                    is_error: true,
                }
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes());
            // dropping stdin closes it (EOF)
        }

        // Drain stdout on a thread so the pipe never blocks the child.
        let stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut out) = stdout {
                use std::io::Read;
                let _ = out.read_to_string(&mut buf);
            }
            buf
        });

        // Poll for completion up to the deadline, then SIGKILL.
        let deadline = std::time::Instant::now() + timeout;
        let mut killed = false;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        killed = true;
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(_) => break,
            }
        }

        let out = reader.join().unwrap_or_default();
        parse_stream_json(&out, killed)
    }
}

/// Parse the final `result` event out of Claude Code's stream-json stdout.
fn parse_stream_json(stdout: &str, killed: bool) -> JobOutcome {
    if killed {
        return JobOutcome {
            result: "Délai dépassé (30 min) — exécution interrompue.".to_string(),
            is_error: true,
        };
    }
    let mut last_result: Option<JobOutcome> = None;
    for line in stdout.lines() {
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
                });
            }
        }
    }
    last_result.unwrap_or(JobOutcome {
        result: if stdout.trim().is_empty() {
            "Claude Code n'a produit aucune sortie.".to_string()
        } else {
            stdout.trim().chars().take(2000).collect()
        },
        is_error: true,
    })
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
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
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
