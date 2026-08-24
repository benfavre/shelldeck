//! Native client for Monique's authenticated Automonique dashboard API.
//!
//! Monique owns the production runtime and exposes typed status, process,
//! operations and conversation endpoints. ShellDeck has no alternate bot
//! transport or fallback when this client cannot connect.

use crate::error::{Result, ShellDeckError};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::redirect::Policy;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(15);
const CHAT_TIMEOUT: Duration = Duration::from_secs(150);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub pass: String,
}

impl MoniqueConfig {
    /// A partially populated Basic-auth endpoint must not win over a complete
    /// server-delivered configuration.
    pub fn is_set(&self) -> bool {
        !self.url.trim().is_empty()
            && !self.user.trim().is_empty()
            && !self.pass.is_empty()
            && self
                .url
                .parse::<reqwest::Url>()
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
    }

    pub fn resolve_effective(
        local: Option<&MoniqueConfig>,
        server: Option<&MoniqueConfig>,
    ) -> Option<MoniqueConfig> {
        local
            .filter(|config| config.is_set())
            .or_else(|| server.filter(|config| config.is_set()))
            .cloned()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueStatus {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub running: Option<u64>,
    #[serde(default)]
    pub inbox_pending: Option<u64>,
    #[serde(default)]
    pub outbox_pending: Option<u64>,
    #[serde(default)]
    pub reconciliation_pending: Option<u64>,
    #[serde(default)]
    pub outbox_ambiguous: Option<u64>,
    #[serde(default)]
    pub provider_available: Option<bool>,
    #[serde(default)]
    pub accepting_intake: Option<bool>,
    #[serde(default)]
    pub generation: Option<u64>,
    #[serde(default)]
    pub execution_state: Option<String>,
    #[serde(default)]
    pub telegram_state: Option<String>,
    #[serde(default)]
    pub observed_ms: Option<u64>,
    #[serde(default)]
    pub stale: bool,
}

impl MoniqueStatus {
    pub fn ready(&self) -> bool {
        self.state == "ready"
            && !self.stale
            && self.provider_available == Some(true)
            && self.accepting_intake == Some(true)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueProcessStats {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub queued: u64,
    #[serde(default)]
    pub running: u64,
    #[serde(default)]
    pub completed: u64,
    #[serde(default)]
    pub failed: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueProcessOutput {
    #[serde(default)]
    pub at_ms: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueProcess {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub issue_id: Option<String>,
    #[serde(default)]
    pub issue_url: Option<String>,
    #[serde(default)]
    pub manage_url: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub runtime: String,
    #[serde(default)]
    pub assigned_to_worker: bool,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub output: Vec<MoniqueProcessOutput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueProcesses {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub observed_at_ms: u64,
    #[serde(default)]
    pub stats: MoniqueProcessStats,
    #[serde(default)]
    pub jobs: Vec<MoniqueProcess>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueChatAction {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub impact: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueChatMessage {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueChatHistory {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub messages: Vec<MoniqueChatMessage>,
    #[serde(default)]
    pub pending_actions: Vec<MoniqueChatAction>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueChatResponse {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub memory_evidence: usize,
    #[serde(default)]
    pub live_sources: Vec<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub conversation_retained: bool,
    #[serde(default)]
    pub action: Option<MoniqueChatAction>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueAgentProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub native_subscription: String,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub account_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueAgentAccount {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub provider_name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub worker_selected: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
    #[serde(default)]
    pub last_verified_at_ms: Option<u64>,
}

impl MoniqueAgentAccount {
    pub fn can_select(&self) -> bool {
        matches!(
            self.status.as_str(),
            "authenticated" | "configured_unverified"
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueAgentLoginSession {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub authorization_url: Option<String>,
    #[serde(default)]
    pub user_code: Option<String>,
    #[serde(default)]
    pub accepts_authorization_code: bool,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub expires_at_ms: u64,
}

impl MoniqueAgentLoginSession {
    pub fn active(&self) -> bool {
        !matches!(
            self.status.as_str(),
            "authenticated" | "failed" | "cancelled"
        )
    }

    pub fn safe_authorization_url(&self) -> Option<&str> {
        let value = self.authorization_url.as_deref()?;
        let url = value.parse::<reqwest::Url>().ok()?;
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        match self.provider.as_str() {
            "codex"
                if url.host_str() == Some("auth.openai.com")
                    && url.path() == "/codex/device"
                    && url.query().is_none() =>
            {
                Some(value)
            }
            "claude"
                if url.host_str() == Some("claude.com")
                    && url.path() == "/cai/oauth/authorize"
                    && url.query().is_some() =>
            {
                Some(value)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoniqueAgentAccounts {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub max_accounts: usize,
    #[serde(default)]
    pub providers: Vec<MoniqueAgentProvider>,
    #[serde(default)]
    pub worker_provider: Option<String>,
    #[serde(default)]
    pub accounts: Vec<MoniqueAgentAccount>,
    #[serde(default)]
    pub login_sessions: Vec<MoniqueAgentLoginSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MoniqueAgentAuthAction {
    StartLogin {
        provider: String,
        label: String,
        account_id: Option<String>,
    },
    Select {
        account_id: String,
    },
    Refresh {
        account_id: String,
    },
    CancelLogin {
        session_id: String,
    },
    SubmitAuthorizationCode {
        session_id: String,
        code: String,
    },
    Logout {
        account_id: String,
        confirm: bool,
    },
    Remove {
        account_id: String,
        confirm: bool,
    },
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    #[serde(default)]
    error: String,
}

fn client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        // Never forward dashboard Basic credentials through a redirect. The
        // canonical Monique URL must be configured explicitly.
        .redirect(Policy::none())
        .build()
        .map_err(|error| ShellDeckError::Connection(format!("Monique HTTP client: {error}")))
}

fn endpoint(config: &MoniqueConfig, path: &str) -> Result<String> {
    if !config.is_set() || !path.starts_with('/') {
        return Err(ShellDeckError::Config(
            "Monique requires a complete http(s) URL and Basic-auth credentials".to_string(),
        ));
    }
    Ok(format!("{}{}", config.url.trim_end_matches('/'), path))
}

fn authenticated(config: &MoniqueConfig, request: RequestBuilder) -> RequestBuilder {
    request.basic_auth(&config.user, Some(&config.pass))
}

fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        let category = response
            .json::<ErrorResponse>()
            .ok()
            .map(|body| body.error)
            .filter(|error| !error.trim().is_empty());
        let detail = category.map_or_else(
            || format!("HTTP {}", status.as_u16()),
            |category| format!("HTTP {} ({category})", status.as_u16()),
        );
        return Err(ShellDeckError::Connection(format!(
            "Monique rejected the request: {detail}"
        )));
    }
    response.json::<T>().map_err(|error| {
        ShellDeckError::Serialization(format!("invalid Monique response: {error}"))
    })
}

fn get<T: DeserializeOwned>(config: &MoniqueConfig, path: &str) -> Result<T> {
    let http = client(READ_TIMEOUT)?;
    let response = authenticated(config, http.get(endpoint(config, path)?))
        .send()
        .map_err(|error| ShellDeckError::Connection(format!("Monique is unreachable: {error}")))?;
    decode(response)
}

fn post<B: Serialize, T: DeserializeOwned>(
    config: &MoniqueConfig,
    path: &str,
    body: &B,
) -> Result<T> {
    let http = client(CHAT_TIMEOUT)?;
    let response = authenticated(config, http.post(endpoint(config, path)?))
        .json(body)
        .send()
        .map_err(|error| ShellDeckError::Connection(format!("Monique is unreachable: {error}")))?;
    decode(response)
}

pub fn status(config: &MoniqueConfig) -> Result<MoniqueStatus> {
    get(config, "/api/status")
}

pub fn processes(config: &MoniqueConfig) -> Result<MoniqueProcesses> {
    get(config, "/api/processes")
}

pub fn chat_history(config: &MoniqueConfig) -> Result<MoniqueChatHistory> {
    get(config, "/api/chat/history")
}

pub fn chat(config: &MoniqueConfig, message: &str) -> Result<MoniqueChatResponse> {
    post(
        config,
        "/api/chat",
        &serde_json::json!({ "message": message, "profile": "conversation" }),
    )
}

pub fn resolve_action(
    config: &MoniqueConfig,
    action_id: &str,
    approved: bool,
) -> Result<MoniqueChatResponse> {
    post(
        config,
        "/api/chat/action",
        &serde_json::json!({
            "action_id": action_id,
            "decision": if approved { "approve" } else { "reject" },
        }),
    )
}

pub fn new_chat(config: &MoniqueConfig) -> Result<MoniqueChatHistory> {
    post(config, "/api/chat/new", &serde_json::json!({}))
}

pub fn agent_accounts(config: &MoniqueConfig) -> Result<MoniqueAgentAccounts> {
    get(config, "/api/agent-accounts")
}

pub fn mutate_agent_accounts(
    config: &MoniqueConfig,
    action: &MoniqueAgentAuthAction,
) -> Result<MoniqueAgentAccounts> {
    post(config, "/api/agent-accounts/action", action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct Mock {
        url: String,
        requests: Arc<Mutex<Vec<(String, String)>>>,
        _thread: std::thread::JoinHandle<()>,
    }

    fn start_mock() -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let thread = std::thread::spawn(move || {
            for _ in 0..8 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut auth = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            auth = value.trim().to_string();
                        } else if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let target = first.split_whitespace().nth(1).unwrap_or("").to_string();
                captured
                    .lock()
                    .unwrap()
                    .push((target.clone(), String::from_utf8_lossy(&body).into_owned()));
                let authorized = auth == "Basic b3BzOnNlY3JldA==";
                let (status, payload) = if !authorized {
                    ("401 Unauthorized", r#"{"error":"unauthorized"}"#)
                } else if target == "/api/status" {
                    ("200 OK", STATUS)
                } else if target == "/api/processes" {
                    ("200 OK", PROCESSES)
                } else if target == "/api/chat/history" || target == "/api/chat/new" {
                    ("200 OK", HISTORY)
                } else if target == "/api/agent-accounts" || target == "/api/agent-accounts/action"
                {
                    ("200 OK", ACCOUNTS)
                } else {
                    ("200 OK", CHAT)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Mock {
            url: format!("http://{address}"),
            requests,
            _thread: thread,
        }
    }

    fn config(mock: &Mock) -> MoniqueConfig {
        MoniqueConfig {
            url: mock.url.clone(),
            user: "ops".into(),
            pass: "secret".into(),
        }
    }

    const STATUS: &str = r#"{"schema":"automonique.dashboard.status/v1","health":"ready","state":"ready","running":1,"inbox_pending":0,"outbox_pending":0,"reconciliation_pending":0,"provider_available":true,"accepting_intake":true,"generation":42,"stale":false}"#;
    const PROCESSES: &str = r#"{"schema":"automonique.dashboard.processes/v1","health":"ready","observed_at_ms":42,"stats":{"total":1,"queued":0,"running":1,"completed":0,"failed":0},"worker":null,"jobs":[{"id":"job-1","status":"running","source":"manage","provider":"codex","runtime":"worker","assigned_to_worker":true,"approved":true,"output":[{"at_ms":42,"kind":"progress","text":"Checking","truncated":false}]}]}"#;
    const HISTORY: &str = r#"{"schema":"automonique.dashboard.chat-history/v1","messages":[{"role":"user","content":"hello","created_at_ms":41}],"pending_actions":[]}"#;
    const CHAT: &str = r#"{"schema":"automonique.dashboard.chat/v2","answer":"Hello from Monique","profile":"conversation","memory_evidence":0,"live_sources":["automonique:status"],"duration_ms":3,"conversation_retained":true,"action":{"id":"action-1","title":"Review action","detail":"Deploy one site","impact":"Changes Manage"}}"#;
    const ACCOUNTS: &str = r#"{"schema":"automonique.dashboard.agent-accounts/v1","max_accounts":64,"providers":[{"id":"codex","name":"Codex CLI","native_subscription":"ChatGPT subscription","available":true,"account_count":2},{"id":"claude","name":"Claude Code","native_subscription":"Claude.ai subscription","available":true,"account_count":1}],"worker_provider":"codex","accounts":[{"id":"acct-0123456789abcdef01234567","provider":"codex","provider_name":"Codex CLI","label":"Codex Pro","selected":true,"worker_selected":true,"status":"authenticated","method":"chatgpt","evidence":"execution_succeeded","observed_at_ms":42,"last_verified_at_ms":42},{"id":"acct-abcdef0123456789abcdef01","provider":"codex","provider_name":"Codex CLI","label":"Codex Team","selected":false,"worker_selected":false,"status":"configured_unverified","method":"chatgpt","evidence":"local_session_present","observed_at_ms":42,"last_verified_at_ms":null},{"id":"acct-111111111111111111111111","provider":"claude","provider_name":"Claude Code","label":"Claude Max","selected":true,"worker_selected":false,"status":"authenticated","method":"claude_ai","evidence":"native_login_verified","observed_at_ms":42,"last_verified_at_ms":42}],"login_sessions":[]}"#;

    #[test]
    // SDTEST-1662
    fn sdtest_monique_contract_reads_runtime_and_conversation() {
        let mock = start_mock();
        let config = config(&mock);
        assert!(status(&config).unwrap().ready());
        let processes = processes(&config).unwrap();
        assert_eq!(processes.jobs[0].output[0].text, "Checking");
        assert_eq!(chat_history(&config).unwrap().messages[0].content, "hello");
        let response = chat(&config, "deploy the site").unwrap();
        assert_eq!(response.answer, "Hello from Monique");
        assert_eq!(response.action.unwrap().id, "action-1");

        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests[3].0, "/api/chat");
        let body: serde_json::Value = serde_json::from_str(&requests[3].1).unwrap();
        assert_eq!(body["message"], "deploy the site");
        assert_eq!(body["profile"], "conversation");
    }

    #[test]
    // SDTEST-1663
    fn sdtest_monique_contract_sends_explicit_action_decisions() {
        let mock = start_mock();
        let config = config(&mock);
        resolve_action(&config, "action-1", true).unwrap();
        new_chat(&config).unwrap();
        let requests = mock.requests.lock().unwrap();
        let decision: serde_json::Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(requests[0].0, "/api/chat/action");
        assert_eq!(decision["action_id"], "action-1");
        assert_eq!(decision["decision"], "approve");
        assert_eq!(requests[1].0, "/api/chat/new");
    }

    #[test]
    // SDTEST-1664
    fn sdtest_monique_config_requires_complete_credentials_and_prefers_local() {
        let partial = MoniqueConfig {
            url: "https://monique.example".into(),
            user: "ops".into(),
            pass: String::new(),
        };
        let server = MoniqueConfig {
            url: "https://server.example".into(),
            user: "ops".into(),
            pass: "server-secret".into(),
        };
        assert!(!partial.is_set());
        assert_eq!(
            MoniqueConfig::resolve_effective(Some(&partial), Some(&server))
                .unwrap()
                .url,
            server.url
        );
    }

    #[test]
    // SDTEST-1665
    fn sdtest_monique_auth_failure_is_explicit() {
        let mock = start_mock();
        let mut config = config(&mock);
        config.pass = "wrong".into();
        let error = status(&config).unwrap_err().to_string();
        assert!(error.contains("401"));
        assert!(error.contains("unauthorized"));
    }

    #[test]
    // SDTEST-1669
    fn sdtest_monique_native_accounts_preserve_n_provider_profiles() {
        let mock = start_mock();
        let config = config(&mock);
        let accounts = agent_accounts(&config).unwrap();
        assert_eq!(accounts.accounts.len(), 3);
        assert_eq!(accounts.max_accounts, 64);
        assert_eq!(accounts.providers[0].account_count, 2);
        assert_eq!(
            accounts
                .accounts
                .iter()
                .filter(|item| item.provider == "codex")
                .count(),
            2
        );
        assert!(accounts.accounts[0].worker_selected);
        assert!(accounts.accounts[1].can_select());
        assert!(accounts
            .login_sessions
            .iter()
            .all(|session| !session.active()));
    }

    #[test]
    // SDTEST-1670
    fn sdtest_monique_native_account_mutations_are_typed_and_token_free() {
        let mock = start_mock();
        let config = config(&mock);
        mutate_agent_accounts(
            &config,
            &MoniqueAgentAuthAction::StartLogin {
                provider: "claude".into(),
                label: "Claude Team".into(),
                account_id: None,
            },
        )
        .unwrap();
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/api/agent-accounts/action");
        let body: serde_json::Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["action"], "start_login");
        assert_eq!(body["provider"], "claude");
        assert_eq!(body["label"], "Claude Team");
        assert!(body.get("token").is_none());
        assert!(body.get("path").is_none());
    }

    #[test]
    // SDTEST-1671
    fn sdtest_monique_native_authorization_links_are_exactly_allowlisted() {
        let mut session = MoniqueAgentLoginSession {
            provider: "codex".into(),
            authorization_url: Some("https://auth.openai.com/codex/device".into()),
            ..Default::default()
        };
        assert!(session.safe_authorization_url().is_some());
        session.authorization_url =
            Some("https://auth.openai.com.attacker.invalid/codex/device".into());
        assert!(session.safe_authorization_url().is_none());
        session.provider = "claude".into();
        session.authorization_url =
            Some("https://claude.com/cai/oauth/authorize?state=opaque".into());
        assert!(session.safe_authorization_url().is_some());
        session.authorization_url = Some("https://claude.com/cai/oauth/authorize".into());
        assert!(session.safe_authorization_url().is_none());
    }

    #[test]
    // SDTEST-1672
    fn sdtest_monique_preserves_multiple_simultaneous_native_login_sessions() {
        let accounts: MoniqueAgentAccounts = serde_json::from_str(
            r#"{
                "schema":"automonique.dashboard.agent-accounts/v1",
                "max_accounts":64,
                "providers":[],
                "accounts":[],
                "login_sessions":[
                    {"id":"login-0123456789abcdef01234567","account_id":"acct-0123456789abcdef01234567","provider":"codex","status":"awaiting_user","authorization_url":"https://auth.openai.com/codex/device","user_code":"ABCD-12345","accepts_authorization_code":false,"created_at_ms":1,"expires_at_ms":2},
                    {"id":"login-abcdef0123456789abcdef01","account_id":"acct-abcdef0123456789abcdef01","provider":"claude","status":"awaiting_user","authorization_url":"https://claude.com/cai/oauth/authorize?state=opaque","user_code":null,"accepts_authorization_code":true,"created_at_ms":1,"expires_at_ms":2}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(accounts.login_sessions.len(), 2);
        assert!(accounts
            .login_sessions
            .iter()
            .all(MoniqueAgentLoginSession::active));
        assert!(accounts
            .login_sessions
            .iter()
            .all(|session| session.safe_authorization_url().is_some()));
        assert_ne!(accounts.login_sessions[0].id, accounts.login_sessions[1].id);
    }
}
