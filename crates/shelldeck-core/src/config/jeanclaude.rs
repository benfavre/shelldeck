//! JeanClaude client — ShellDeck as a native front-end for Ben's `#jean` Slack
//! ticket bot (repo `slack-claude-bot`), replacing its web dashboard.
//!
//! The bot exposes a Basic-Auth JSON control API on its dashboard server
//! (`src/dashboard.ts`). Shapes here are derived from that source (getState in
//! `index.ts`, the `Ticket`/`Ignored` records in `registry.ts`, `Memory` in
//! `memory.ts`, targets `{suffixes,mappings}` of `{sshHost,note}`). Parsing is
//! defensive (nullable strings, optional numbers) since records come straight
//! from SQLite.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Deserialize a string the server may send as JSON `null` → `""`.
fn de_nullable_string<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// JeanClaude dashboard connection config. Comes from either the sites payload
/// (`SitesPayload.jeanclaude`, super-admin only) or a local `[jeanclaude]`
/// section of `shelldeck.toml` (which takes precedence).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JeanConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub pass: String,
}

impl JeanConfig {
    /// A usable config needs at least a URL.
    pub fn is_set(&self) -> bool {
        !self.url.trim().is_empty()
    }

    /// Pure resolver: local `[jeanclaude]` in `shelldeck.toml` wins when
    /// set; otherwise fall back to the server-delivered config (super-admin
    /// only, per AGENTS.md § JeanClaude). Neither set ⇒ feature unavailable.
    ///
    /// This is the pure-logic port of `Workspace::effective_jean_config`.
    /// The GPUI method delegates; unit tests hit this directly without a
    /// `Context`.
    pub fn resolve_effective(
        local: Option<&JeanConfig>,
        server: Option<&JeanConfig>,
    ) -> Option<JeanConfig> {
        if let Some(l) = local {
            if l.is_set() {
                return Some(l.clone());
            }
        }
        server.filter(|s| s.is_set()).cloned()
    }
}

/// Format a JeanClaude `say` message with the "via ShellDeck" origin
/// prefix, so a bot admin reading Slack knows the message came from a
/// ShellDeck operator (and which one, by display name).
///
/// Contract: `"[via ShellDeck — {name}] {text}"`. Consumed by
/// `Workspace::send_jean_ask` and the Support "Envoyer à Jean" flow.
/// Pinned so a copy-paste refactor doesn't drop the brackets or the
/// em-dash (SDTEST-246).
pub fn format_via_shelldeck(name: &str, text: &str) -> String {
    format!("[via ShellDeck — {}] {}", name, text)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanBot {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub max: i64,
    #[serde(default)]
    pub started_at: f64,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub bot_user: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub workspace: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub channel: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub channel_name: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub permission_mode: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub work_dir: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanSettings {
    #[serde(default)]
    pub confirmers: Vec<String>,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub classify_enabled: bool,
    #[serde(default)]
    pub split_enabled: bool,
    #[serde(default)]
    pub max_concurrency: i64,
}

/// A ticket record (audit trail + live activity). `actions` is only present in
/// the `/api/ticket` detail response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanTicket {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub channel: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub thread_ts: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub confirmed_by: Option<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub prompt: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    #[serde(default)]
    pub queued_at: f64,
    #[serde(default)]
    pub started_at: Option<f64>,
    #[serde(default)]
    pub ended_at: Option<f64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub num_turns: Option<i64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// SSH host ("Cible").
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub ticket_url: Option<String>,
    #[serde(default)]
    pub site_url: Option<String>,
    #[serde(default)]
    pub server_ip: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Live: what the worker is doing right now.
    #[serde(default)]
    pub activity: Option<String>,
    /// Live: last stream event time (ms). Drives the heartbeat-age warning.
    #[serde(default)]
    pub last_activity_at: Option<f64>,
    /// Detail-only: full rolling action log.
    #[serde(default)]
    pub actions: Vec<String>,
}

impl JeanTicket {
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
    pub fn is_queued(&self) -> bool {
        self.status == "queued"
    }
    /// Heartbeat age in ms relative to `now_ms`, if running.
    pub fn heartbeat_age_ms(&self, now_ms: f64) -> Option<f64> {
        self.last_activity_at.map(|t| (now_ms - t).max(0.0))
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanIgnored {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub channel: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub thread_ts: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub text: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub reason: String,
    #[serde(default)]
    pub at: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanPending {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub thread_ts: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub channel: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub prompt: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub author_name: Option<String>,
    /// Split parts, if this pending item is several tickets in one message.
    #[serde(default)]
    pub parts: Option<Vec<String>>,
    #[serde(default = "one")]
    pub count: i64,
    #[serde(default)]
    pub at: f64,
}

fn one() -> i64 {
    1
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct JeanState {
    #[serde(default)]
    pub bot: JeanBot,
    #[serde(default)]
    pub settings: JeanSettings,
    #[serde(default)]
    pub tickets: Vec<JeanTicket>,
    #[serde(default)]
    pub ignored: Vec<JeanIgnored>,
    #[serde(default)]
    pub pending: Vec<JeanPending>,
}

/// A memory rule: `notify` (@mention on match), `note` (fact injected), or
/// `usermap` (github login → slack id).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanMemory {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub kind: String,
    #[serde(rename = "match", default, deserialize_with = "de_nullable_string")]
    pub match_: String,
    #[serde(default)]
    pub notify_ids: Vec<String>,
    #[serde(default)]
    pub notify_names: Vec<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub text: String,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub created_at: f64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanTargetRule {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub ssh_host: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct JeanTargets {
    #[serde(default)]
    pub suffixes: BTreeMap<String, JeanTargetRule>,
    #[serde(default)]
    pub mappings: BTreeMap<String, JeanTargetRule>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JeanSlackMsg {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub ts: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub is_system: bool,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub text: String,
    #[serde(default)]
    pub replies: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ActionResponse {
    #[serde(default)]
    ok: bool,
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

fn url_for(cfg: &JeanConfig, path: &str) -> String {
    format!("{}{}", cfg.url.trim_end_matches('/'), path)
}

fn check_status(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 => Err(ShellDeckError::Connection(
            "identifiants JeanClaude rejetés (401)".to_string(),
        )),
        s => Err(ShellDeckError::Connection(format!(
            "JeanClaude a répondu HTTP {}",
            s
        ))),
    }
}

fn get_json<T: for<'de> Deserialize<'de>>(cfg: &JeanConfig, path: &str) -> Result<T> {
    let client = http_client()?;
    let resp = client
        .get(url_for(cfg, path))
        .basic_auth(&cfg.user, Some(&cfg.pass))
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("JeanClaude injoignable : {}", e)))?;
    check_status(resp.status().as_u16())?;
    resp.json::<T>()
        .map_err(|e| ShellDeckError::Serialization(format!("réponse JeanClaude invalide : {}", e)))
}

/// POST a control action; surface the server's `error` string on `ok:false`.
fn post_action(cfg: &JeanConfig, path: &str, body: serde_json::Value) -> Result<()> {
    let client = http_client()?;
    let resp = client
        .post(url_for(cfg, path))
        .basic_auth(&cfg.user, Some(&cfg.pass))
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("JeanClaude injoignable : {}", e)))?;
    check_status(resp.status().as_u16())?;
    let parsed: ActionResponse = resp.json().unwrap_or(ActionResponse {
        ok: true,
        error: None,
    });
    if parsed.ok {
        Ok(())
    } else {
        Err(ShellDeckError::Connection(
            parsed.error.unwrap_or_else(|| "action refusée".to_string()),
        ))
    }
}

// ── reads ────────────────────────────────────────────────────────────────

pub fn get_state(cfg: &JeanConfig) -> Result<JeanState> {
    get_json(cfg, "/api/state")
}

pub fn get_history(cfg: &JeanConfig, q: &str, status: &str, limit: u32) -> Result<Vec<JeanTicket>> {
    let path = format!(
        "/api/history?q={}&status={}&limit={}",
        crate::config::cloud_account::percent_encode(q),
        crate::config::cloud_account::percent_encode(status),
        limit
    );
    get_json(cfg, &path)
}

pub fn get_ticket(cfg: &JeanConfig, id: &str) -> Result<JeanTicket> {
    get_json(
        cfg,
        &format!(
            "/api/ticket?id={}",
            crate::config::cloud_account::percent_encode(id)
        ),
    )
}

pub fn get_targets(cfg: &JeanConfig) -> Result<JeanTargets> {
    get_json(cfg, "/api/targets")
}

pub fn get_memory(cfg: &JeanConfig) -> Result<Vec<JeanMemory>> {
    get_json(cfg, "/api/memory")
}

pub fn get_slack_history(cfg: &JeanConfig) -> Result<Vec<JeanSlackMsg>> {
    get_json(cfg, "/api/slack/history")
}

// ── writes ───────────────────────────────────────────────────────────────

pub fn confirm(cfg: &JeanConfig, thread_ts: &str) -> Result<()> {
    post_action(
        cfg,
        "/api/confirm",
        serde_json::json!({ "threadTs": thread_ts }),
    )
}

pub fn reject(cfg: &JeanConfig, thread_ts: &str) -> Result<()> {
    post_action(
        cfg,
        "/api/reject",
        serde_json::json!({ "threadTs": thread_ts }),
    )
}

pub fn cancel(cfg: &JeanConfig, id: &str) -> Result<()> {
    post_action(cfg, "/api/cancel", serde_json::json!({ "id": id }))
}

pub fn force_ticket(cfg: &JeanConfig, id: &str) -> Result<()> {
    post_action(cfg, "/api/force-ticket", serde_json::json!({ "id": id }))
}

pub fn set_paused(cfg: &JeanConfig, paused: bool) -> Result<()> {
    post_action(cfg, "/api/pause", serde_json::json!({ "paused": paused }))
}

pub fn set_concurrency(cfg: &JeanConfig, max: i64) -> Result<()> {
    post_action(cfg, "/api/concurrency", serde_json::json!({ "max": max }))
}

pub fn say(cfg: &JeanConfig, text: &str) -> Result<()> {
    post_action(cfg, "/api/say", serde_json::json!({ "text": text }))
}

pub fn add_target(cfg: &JeanConfig, domain: &str, ssh_host: &str, note: &str) -> Result<()> {
    post_action(
        cfg,
        "/api/targets/add",
        serde_json::json!({ "domain": domain, "sshHost": ssh_host, "note": note }),
    )
}

pub fn remove_target(cfg: &JeanConfig, domain: &str) -> Result<()> {
    post_action(
        cfg,
        "/api/targets/remove",
        serde_json::json!({ "domain": domain }),
    )
}

pub fn add_memory(
    cfg: &JeanConfig,
    kind: &str,
    match_: &str,
    notify_ids: &[String],
    text: &str,
) -> Result<()> {
    post_action(
        cfg,
        "/api/memory/add",
        serde_json::json!({ "kind": kind, "match": match_, "notifyIds": notify_ids, "text": text }),
    )
}

pub fn remove_memory(cfg: &JeanConfig, id: &str) -> Result<()> {
    post_action(cfg, "/api/memory/remove", serde_json::json!({ "id": id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    // Minimal base64 (standard alphabet, padded) so the mock can validate the
    // exact Basic-auth header the client sends.
    fn b64(input: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let b0 = chunk[0];
            let b1 = *chunk.get(1).unwrap_or(&0);
            let b2 = *chunk.get(2).unwrap_or(&0);
            out.push(T[(b0 >> 2) as usize] as char);
            out.push(T[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            out.push(if chunk.len() > 1 {
                T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                T[(b2 & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    // A tiny canned-response HTTP mock that validates the Basic-auth header
    // (jean:secret) and echoes a fixture per path, else 401.
    struct Mock {
        url: String,
        _handle: std::thread::JoinHandle<()>,
    }

    fn start_mock() -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected = format!("Basic {}", b64(b"jean:secret"));
        let handle = std::thread::spawn(move || {
            for _ in 0..48 {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let mut auth = String::new();
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let t = line.trim_end();
                    if t.is_empty() {
                        break;
                    }
                    if let Some(idx) = t.find(':') {
                        let key = t[..idx].trim().to_ascii_lowercase();
                        let val = t[idx + 1..].trim();
                        if key == "authorization" {
                            auth = val.to_string();
                        } else if key == "content-length" {
                            content_length = val.parse().unwrap_or(0);
                        }
                    }
                }
                if content_length > 0 {
                    let mut body = vec![0u8; content_length];
                    let _ = reader.read_exact(&mut body);
                }

                let target = request_line.split_whitespace().nth(1).unwrap_or("");
                let path = target.split('?').next().unwrap_or("");

                let (status, body): (u16, String) = if auth != expected {
                    (401, r#"{"ok":false,"error":"unauthorized"}"#.to_string())
                } else {
                    match path {
                        "/api/state" => (200, STATE_FIXTURE.to_string()),
                        "/api/history" => (200, HISTORY_FIXTURE.to_string()),
                        "/api/ticket" => (200, TICKET_FIXTURE.to_string()),
                        "/api/targets" => (200, TARGETS_FIXTURE.to_string()),
                        "/api/memory" => (200, MEMORY_FIXTURE.to_string()),
                        "/api/force-ticket" => {
                            (200, r#"{"ok":false,"error":"introuvable"}"#.to_string())
                        }
                        _ => (200, r#"{"ok":true}"#.to_string()),
                    }
                };

                let reason = if status == 401 { "Unauthorized" } else { "OK" };
                let resp = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    reason,
                    body.as_bytes().len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        Mock {
            url: format!("http://127.0.0.1:{}", port),
            _handle: handle,
        }
    }

    fn cfg(m: &Mock) -> JeanConfig {
        JeanConfig {
            url: m.url.clone(),
            user: "jean".to_string(),
            pass: "secret".to_string(),
        }
    }

    // Fixtures derived from the bot source (registry.ts / index.ts getState).
    const STATE_FIXTURE: &str = r#"{
      "bot": { "connected": true, "paused": false, "max": 2, "startedAt": 1751470000000,
               "botUser": "U123", "workspace": "T1", "channel": "C1", "channelName": "jean",
               "permissionMode": "acceptEdits", "workDir": "/home/x/infra" },
      "settings": { "confirmers": ["U1"], "allowedChannels": ["C1"], "allowedUsers": [],
                    "classifyEnabled": true, "splitEnabled": true, "maxConcurrency": 2 },
      "names": { "U1": "Jordan" },
      "tickets": [
        { "id": "t3", "channel": "C1", "threadTs": "1.2", "author": "U9", "authorName": "Bob",
          "prompt": "corrige le hero", "status": "running", "queuedAt": 1751460000000,
          "startedAt": 1751461000000, "target": "activ-2", "activity": "édition du fichier",
          "lastActivityAt": 1751469990000 },
        { "id": "t2", "channel": "C1", "threadTs": "1.1", "author": null, "confirmedBy": "U1",
          "prompt": "autre", "status": "done", "queuedAt": 1751450000000, "endedAt": 1751451000000,
          "numTurns": 7, "costUsd": 0.42, "ticketUrl": "https://github.com/x/y/issues/3" }
      ],
      "ignored": [
        { "id": "i1", "channel": "C1", "threadTs": "1.0", "author": "U9", "authorName": "Bob",
          "text": "lol", "reason": "chatter", "at": 1751440000000 }
      ],
      "pending": [
        { "threadTs": "1.3", "channel": "C1", "prompt": "déploie le site", "author": "U9",
          "authorName": "Bob", "parts": null, "count": 1, "at": 1751469000000 }
      ]
    }"#;

    const HISTORY_FIXTURE: &str = r#"[
      { "id": "t2", "channel": "C1", "threadTs": "1.1", "prompt": "autre", "status": "done",
        "queuedAt": 1751450000000, "authorName": "Bob", "costUsd": 0.42 },
      { "id": "t1", "channel": "C1", "threadTs": "1.0", "prompt": "premier", "status": "error",
        "queuedAt": 1751430000000, "error": "boom", "author": null }
    ]"#;

    const TICKET_FIXTURE: &str = r#"{
      "id": "t3", "channel": "C1", "threadTs": "1.2", "author": "U9", "prompt": "corrige le hero",
      "status": "running", "queuedAt": 1751460000000, "target": "activ-2",
      "actions": ["🎯 cible activ-2", "édition src/hero.tsx", "déploiement"]
    }"#;

    const TARGETS_FIXTURE: &str = r#"{
      "suffixes": { ".bext.dev": { "sshHost": "activ-2", "note": "prism" } },
      "mappings": { "example.com": { "sshHost": "web-1", "note": null } }
    }"#;

    const MEMORY_FIXTURE: &str = r#"[
      { "id": "m2", "kind": "notify", "match": "seo", "notifyIds": ["U1"], "notifyNames": ["Jordan"],
        "text": "", "createdAt": 1751400000000 },
      { "id": "m1", "kind": "note", "match": "", "notifyIds": [], "notifyNames": [],
        "text": "toujours en français", "createdBy": "U1", "createdAt": 1751390000000 }
    ]"#;

    #[test]
    fn parse_state() {
        let m = start_mock();
        let s = get_state(&cfg(&m)).expect("state");
        assert!(s.bot.connected && !s.bot.paused);
        assert_eq!(s.bot.max, 2);
        assert_eq!(s.bot.channel_name, "jean");
        assert_eq!(s.settings.confirmers, vec!["U1".to_string()]);
        assert_eq!(s.tickets.len(), 2);
        assert!(s.tickets[0].is_running());
        assert_eq!(s.tickets[0].target.as_deref(), Some("activ-2"));
        assert_eq!(s.tickets[0].activity.as_deref(), Some("édition du fichier"));
        assert!(s.tickets[0].heartbeat_age_ms(1751470000000.0).unwrap() >= 0.0);
        assert!(s.tickets[1].author.is_none()); // null author tolerated
        assert_eq!(s.tickets[1].cost_usd, Some(0.42));
        assert_eq!(s.ignored.len(), 1);
        assert_eq!(s.ignored[0].reason, "chatter");
        assert_eq!(s.pending.len(), 1);
        assert_eq!(s.pending[0].count, 1);
        assert_eq!(s.pending[0].thread_ts, "1.3");
    }

    #[test]
    fn parse_history_ticket_targets_memory() {
        let m = start_mock();
        let c = cfg(&m);
        let h = get_history(&c, "", "", 50).expect("history");
        assert_eq!(h.len(), 2);
        assert_eq!(h[1].status, "error");
        assert_eq!(h[1].error.as_deref(), Some("boom"));

        let t = get_ticket(&c, "t3").expect("ticket");
        assert_eq!(t.actions.len(), 3);
        assert_eq!(t.actions[0], "🎯 cible activ-2");

        let tg = get_targets(&c).expect("targets");
        assert_eq!(tg.suffixes[".bext.dev"].ssh_host, "activ-2");
        assert_eq!(tg.mappings["example.com"].ssh_host, "web-1");
        assert!(tg.mappings["example.com"].note.is_none()); // null note tolerated

        let mem = get_memory(&c).expect("memory");
        assert_eq!(mem.len(), 2);
        assert_eq!(mem[0].kind, "notify");
        assert_eq!(mem[0].match_, "seo");
        assert_eq!(mem[0].notify_names, vec!["Jordan".to_string()]);
    }

    #[test]
    fn post_actions_and_error_surface() {
        let m = start_mock();
        let c = cfg(&m);
        assert!(confirm(&c, "1.3").is_ok());
        assert!(reject(&c, "1.3").is_ok());
        assert!(cancel(&c, "t3").is_ok());
        assert!(set_paused(&c, true).is_ok());
        assert!(say(&c, "bonjour").is_ok());
        // force-ticket returns ok:false → surfaces the server error.
        let err = force_ticket(&c, "nope").unwrap_err();
        assert!(err.to_string().contains("introuvable"));
    }

    #[test]
    fn wrong_credentials_surface_401() {
        let m = start_mock();
        let mut c = cfg(&m);
        // Correct creds → ok.
        assert!(get_state(&c).is_ok());
        // Wrong password → the mock rejects with 401, surfaced clearly.
        c.pass = "nope".to_string();
        let err = get_state(&c).unwrap_err();
        assert!(err.to_string().contains("401"), "got: {}", err);
    }

    #[test]
    fn is_set_semantics() {
        assert!(!JeanConfig::default().is_set());
        assert!(JeanConfig {
            url: "http://127.0.0.1:3100".into(),
            ..Default::default()
        }
        .is_set());
    }

    // SDTEST-1054 — `resolve_effective` precedence per AGENTS.md §
    // JeanClaude: a local `[jeanclaude]` (typically pointing at an SSH
    // tunnel on 127.0.0.1) MUST override the server-delivered config,
    // and an unset local slot MUST fall through to the server one.
    // Neither ⇒ feature unavailable.

    fn set_cfg(url: &str) -> JeanConfig {
        JeanConfig {
            url: url.into(),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_effective_local_wins_over_server() {
        let local = set_cfg("http://127.0.0.1:3100");
        let server = set_cfg("https://jean.manage.example");
        let out = JeanConfig::resolve_effective(Some(&local), Some(&server));
        assert_eq!(out.expect("some").url, "http://127.0.0.1:3100");
    }

    #[test]
    fn resolve_effective_falls_back_to_server_when_local_unset() {
        let local = JeanConfig::default(); // empty url ⇒ !is_set
        let server = set_cfg("https://jean.manage.example");
        let out = JeanConfig::resolve_effective(Some(&local), Some(&server));
        assert_eq!(out.expect("some").url, "https://jean.manage.example");
    }

    #[test]
    fn resolve_effective_falls_back_to_server_when_local_none() {
        let server = set_cfg("https://jean.manage.example");
        let out = JeanConfig::resolve_effective(None, Some(&server));
        assert_eq!(out.expect("some").url, "https://jean.manage.example");
    }

    #[test]
    fn resolve_effective_none_when_neither_set() {
        assert!(JeanConfig::resolve_effective(None, None).is_none());
        assert!(JeanConfig::resolve_effective(Some(&JeanConfig::default()), None).is_none());
        assert!(
            JeanConfig::resolve_effective(None, Some(&JeanConfig::default())).is_none(),
            "server present but empty url ⇒ not available",
        );
    }

    // SDTEST-246 — the "Envoyer à Jean" bridge from Support prefixes
    // every message with `[via ShellDeck — <name>]` so the Slack
    // reader can trace it back to the ShellDeck operator. Contract is
    // deliberately opinionated — square brackets, em-dash (`—`, U+2014),
    // trailing space after the closing bracket. A copy-paste refactor
    // that drops any of those would break the visual pattern the bot
    // team relies on to filter Slack.
    #[test]
    fn format_via_shelldeck_prefix_shape_is_pinned() {
        let out = format_via_shelldeck("Ben", "corrige X");
        assert_eq!(out, "[via ShellDeck — Ben] corrige X");
    }

    #[test]
    fn format_via_shelldeck_empty_name_still_brackets_cleanly() {
        // The Workspace falls back to an empty name for logged-out or
        // anonymous cases. The bracket + em-dash still appear — that's
        // what makes the prefix greppable in the bot's channel filter,
        // even when the operator is unnamed.
        let out = format_via_shelldeck("", "corrige X");
        assert_eq!(out, "[via ShellDeck — ] corrige X");
    }

    #[test]
    fn format_via_shelldeck_preserves_text_verbatim() {
        // Multi-line, unicode, and existing brackets in the payload
        // survive untouched — no escaping, no truncation.
        let text = "ligne 1\nligne 2 [avec crochets] — et un tiret";
        let out = format_via_shelldeck("Alice", text);
        assert_eq!(
            out,
            format!("[via ShellDeck — Alice] {}", text),
            "the text payload is copied byte-for-byte after the prefix",
        );
    }
}
