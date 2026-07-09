//! Hosted issue management (requests) — per tenant/site, GitHub-synced,
//! fleet-dispatchable. Users file requests; support/staff triage them.
//!
//! Endpoint: `{base}/api/manage/shelldeck/issues` (Bearer device token). Shapes
//! are snake_case with ISO-8601 timestamps (like the fleet API), parsed
//! defensively. Staff-only actions (status/assign/priority/dispatch/github)
//! return 403 for non-super-admin tokens.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

fn de_nullable_string<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// Timestamps arrive as ISO-8601 strings (sometimes numbers / null) → epoch ms.
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IssueGithub {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub url: String,
    #[serde(default)]
    pub number: i64,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub state: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IssueComment {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub author: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub body: String,
    /// "comment" | "status" | "system" | "github".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub kind: String,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub at: f64,
}

impl IssueComment {
    pub fn is_note(&self) -> bool {
        matches!(self.kind.as_str(), "status" | "system" | "github")
    }
}

/// An issue. Slim in the list response; the `?action=issue` detail adds `body`,
/// `comments`, and `job_ids` (defaulted empty otherwise).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Issue {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub tenant_id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub tenant_name: String,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub site_label: Option<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub title: String,
    /// "open" | "triaging" | "in_progress" | "blocked" | "done" | "closed".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    /// "low" | "normal" | "high" | "urgent".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub priority: String,
    /// "user" | "support" | "shelldeck" | "slack" | "manage".
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub source: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub requested_by: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub assignee: String,
    #[serde(default)]
    pub comment_count: i64,
    #[serde(default)]
    pub github: Option<IssueGithub>,
    #[serde(default)]
    pub job_count: i64,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub created_at: f64,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub updated_at: f64,
    // detail-only:
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<IssueComment>,
    #[serde(default)]
    pub job_ids: Vec<String>,
}

impl Issue {
    pub fn is_unassigned(&self) -> bool {
        self.assignee.trim().is_empty()
    }
    pub fn is_closed(&self) -> bool {
        matches!(self.status.as_str(), "done" | "closed")
    }
}

/// A fleet instance offered for dispatch (from the list response).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IssueInstance {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub tenant_id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IssueList {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub issues: Vec<Issue>,
    /// Whether the token is staff (super-admin) — drives the triage action bar.
    #[serde(default)]
    pub staff: bool,
    #[serde(default)]
    pub instances: Vec<IssueInstance>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct IssueResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    issue: Issue,
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

fn issues_url(base_url: &str) -> String {
    format!(
        "{}/api/manage/shelldeck/issues",
        base_url.trim_end_matches('/')
    )
}

fn check_status(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 => Err(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        )),
        403 => Err(ShellDeckError::Connection(
            "action réservée au support (403)".to_string(),
        )),
        s => Err(ShellDeckError::Connection(format!(
            "issues request failed: HTTP {}",
            s
        ))),
    }
}

fn get_json<T: for<'de> Deserialize<'de>>(base_url: &str, token: &str, query: &str) -> Result<T> {
    let client = http_client()?;
    let resp = client
        .get(format!("{}{}", issues_url(base_url), query))
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("issues request failed: {}", e)))?;
    check_status(resp.status().as_u16())?;
    resp.json::<T>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid issues payload: {}", e)))
}

/// POST an action, returning the updated issue (surfacing the server error /
/// 403 for staff-gated actions).
fn post_issue(base_url: &str, token: &str, body: serde_json::Value) -> Result<Issue> {
    let client = http_client()?;
    let resp = client
        .post(issues_url(base_url))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("issues request failed: {}", e)))?;
    check_status(resp.status().as_u16())?;
    let parsed: IssueResponse = resp
        .json()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid issue response: {}", e)))?;
    if parsed.ok {
        Ok(parsed.issue)
    } else {
        Err(ShellDeckError::Connection(
            parsed.error.unwrap_or_else(|| "action refusée".to_string()),
        ))
    }
}

// ── reads ────────────────────────────────────────────────────────────────

/// List issues in the token's scope (`status`/`q` optional filters).
pub fn list_issues(base_url: &str, token: &str, status: &str, q: &str) -> Result<IssueList> {
    let query = format!(
        "?action=list&status={}&q={}",
        crate::config::cloud_account::percent_encode(status),
        crate::config::cloud_account::percent_encode(q),
    );
    get_json(base_url, token, &query)
}

/// Fetch one issue with its body + comments + job ids.
pub fn get_issue(base_url: &str, token: &str, id: &str) -> Result<Issue> {
    #[derive(Deserialize)]
    struct One {
        #[serde(default)]
        issue: Issue,
    }
    let one: One = get_json(
        base_url,
        token,
        &format!(
            "?action=issue&id={}",
            crate::config::cloud_account::percent_encode(id)
        ),
    )?;
    Ok(one.issue)
}

// ── writes (anyone) ──────────────────────────────────────────────────────

/// File a new request. `source` = "user" (default) or "support".
pub fn create_issue(
    base_url: &str,
    token: &str,
    title: &str,
    body: &str,
    priority: &str,
    source: &str,
) -> Result<Issue> {
    let mut b = serde_json::json!({ "action": "create", "title": title });
    let obj = b.as_object_mut().unwrap();
    if !body.is_empty() {
        obj.insert("body".into(), serde_json::json!(body));
    }
    if !priority.is_empty() {
        obj.insert("priority".into(), serde_json::json!(priority));
    }
    if !source.is_empty() {
        obj.insert("source".into(), serde_json::json!(source));
    }
    post_issue(base_url, token, b)
}

/// Add a comment (mirrors to GitHub if the issue is linked).
pub fn comment_issue(base_url: &str, token: &str, id: &str, body: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "comment", "id": id, "body": body }),
    )
}

// ── writes (staff only — 403 otherwise) ──────────────────────────────────

pub fn set_status(base_url: &str, token: &str, id: &str, status: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "status", "id": id, "status": status }),
    )
}

pub fn assign(base_url: &str, token: &str, id: &str, assignee: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "assign", "id": id, "assignee": assignee }),
    )
}

pub fn set_priority(base_url: &str, token: &str, id: &str, priority: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "priority", "id": id, "priority": priority }),
    )
}

/// Dispatch the issue to a fleet instance (creates a fleet job, links job_ids).
pub fn dispatch_issue(base_url: &str, token: &str, id: &str, instance_id: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "dispatch", "id": id, "instance_id": instance_id }),
    )
}

/// Create the GitHub issue in the tenant's mapped repo.
pub fn github_push(base_url: &str, token: &str, id: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "github-push", "id": id }),
    )
}

/// Pull GitHub state/comments back into the issue.
pub fn github_refresh(base_url: &str, token: &str, id: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "github-refresh", "id": id }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct Mock {
        url: String,
        posts: Arc<Mutex<Vec<String>>>,
        _handle: std::thread::JoinHandle<()>,
    }

    /// Canned issues mock: Bearer-gated, records POSTs, serves list/detail
    /// fixtures, echoes an updated issue on create/comment, and returns 403 for
    /// staff-only actions.
    fn start_mock() -> Mock {
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
                    if let Some(i) = t.find(':') {
                        let k = t[..i].trim().to_ascii_lowercase();
                        let v = t[i + 1..].trim();
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
                let target = request_line.split_whitespace().nth(1).unwrap_or("");

                let bearer_token = auth.strip_prefix("Bearer ").unwrap_or("").trim();
                let (status, out): (u16, String) = if bearer_token.is_empty() {
                    (401, r#"{"ok":false,"error":"unauthorized"}"#.into())
                } else if method == "GET" && target.contains("action=issue") {
                    (200, DETAIL_FIXTURE.into())
                } else if method == "GET" {
                    (200, LIST_FIXTURE.into())
                } else {
                    posts2.lock().unwrap().push(body.clone());
                    let action = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("action").and_then(|a| a.as_str()).map(String::from))
                        .unwrap_or_default();
                    match action.as_str() {
                        "create" | "comment" => (
                            200,
                            r#"{"ok":true,"issue":{"id":"iss_1","title":"t","status":"open","priority":"normal","source":"user","comments":[{"id":"c1","author":"me","body":"hi","kind":"comment","at":"2026-07-02T21:00:00.000Z"}]}}"#.into(),
                        ),
                        // staff-only actions → 403 for this (non-staff) fixture path
                        "status" | "assign" | "priority" | "dispatch" | "github-push"
                        | "github-refresh" => {
                            (403, r#"{"ok":false,"error":"forbidden"}"#.into())
                        }
                        _ => (200, r#"{"ok":true,"issue":{"id":"iss_1"}}"#.into()),
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

    const LIST_FIXTURE: &str = r#"{
      "ok": true, "staff": true,
      "issues": [
        { "id":"iss_1", "tenant_id":"t1", "tenant_name":"Acme", "site_id":null, "site_label":null,
          "title":"Bug hero", "status":"open", "priority":"high", "source":"user",
          "requested_by":"ben@x.fr", "assignee":"", "comment_count":2,
          "github":{"url":"https://github.com/o/r/issues/7","number":7,"state":"open"},
          "job_count":1, "created_at":"2026-07-02T20:00:00.000Z", "updated_at":"2026-07-02T20:30:00.000Z" },
        { "id":"iss_2", "tenant_id":"t2", "tenant_name":"Beta", "title":"Autre",
          "status":"done", "priority":"normal", "source":"support", "requested_by":"u2",
          "assignee":"agent@x.fr", "comment_count":0, "github":null, "job_count":0,
          "created_at":"2026-07-01T10:00:00.000Z", "updated_at":"2026-07-01T12:00:00.000Z" }
      ],
      "instances": [ { "id":"i1", "name":"activ-2", "tenant_id":"t1", "status":"online" } ]
    }"#;

    const DETAIL_FIXTURE: &str = r#"{
      "ok": true,
      "issue": { "id":"iss_1", "tenant_id":"t1", "tenant_name":"Acme", "title":"Bug hero",
        "status":"open", "priority":"high", "source":"user", "requested_by":"ben@x.fr",
        "assignee":"", "comment_count":2, "job_count":1,
        "github":{"url":"https://github.com/o/r/issues/7","number":7,"state":"open"},
        "created_at":"2026-07-02T20:00:00.000Z", "updated_at":"2026-07-02T20:30:00.000Z",
        "body":"le hero est cassé", "job_ids":["j1"],
        "comments":[
          {"id":"c1","author":"ben","body":"des détails","kind":"comment","at":"2026-07-02T20:05:00.000Z"},
          {"id":"c2","author":"système","body":"statut → in_progress","kind":"status","at":"2026-07-02T20:10:00.000Z"}
        ] }
    }"#;

    fn cfg(m: &Mock) -> (String, String) {
        (m.url.clone(), TOKEN.to_string())
    }

    #[test]
    fn parse_list() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let l = list_issues(&b, &t, "", "").expect("list");
        assert!(l.ok && l.staff);
        assert_eq!(l.issues.len(), 2);
        assert_eq!(l.issues[0].title, "Bug hero");
        assert_eq!(l.issues[0].github.as_ref().unwrap().number, 7);
        assert!(l.issues[0].created_at > 0.0, "ISO created_at parsed");
        assert!(l.issues[0].is_unassigned());
        assert!(l.issues[1].is_closed());
        assert_eq!(l.instances.len(), 1);
        assert_eq!(l.instances[0].name, "activ-2");
    }

    #[test]
    fn parse_detail() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let iss = get_issue(&b, &t, "iss_1").expect("detail");
        assert_eq!(iss.body, "le hero est cassé");
        assert_eq!(iss.comments.len(), 2);
        assert!(!iss.comments[0].is_note());
        assert!(iss.comments[1].is_note()); // "status" kind
        assert_eq!(iss.job_ids, vec!["j1".to_string()]);
    }

    #[test]
    fn create_and_comment_bodies() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let created = create_issue(&b, &t, "Nouveau", "corps", "high", "support").expect("create");
        assert_eq!(created.id, "iss_1");
        comment_issue(&b, &t, "iss_1", "un mot").expect("comment");

        let posts = m.posts.lock().unwrap();
        assert!(posts.iter().any(|p| p.contains("\"action\":\"create\"")
            && p.contains("\"source\":\"support\"")
            && p.contains("\"priority\":\"high\"")));
        assert!(posts.iter().any(|p| p.contains("\"action\":\"comment\"")));
    }

    #[test]
    fn staff_actions_surface_403() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let err = set_status(&b, &t, "iss_1", "done").unwrap_err();
        assert!(err.to_string().contains("403"), "got {}", err);
        assert!(dispatch_issue(&b, &t, "iss_1", "i1").is_err());
    }

    #[test]
    fn missing_bearer_surfaces_401() {
        let m = start_mock();
        let err = list_issues(&m.url, "", "", "").unwrap_err();
        assert!(err.to_string().contains("401"), "got {}", err);
    }

    // SDTEST-298 — `dispatch_issue` body must carry both `id` and
    // `instance_id` under the "dispatch" action. Fleet routing is
    // never exercised live (would spawn a real claude job on a real
    // instance), so this mock-only shape assertion is the only guard
    // against a rename ("target_instance", "instance", "instanceId"…)
    // that would silently 400 in prod.
    //
    // The mock returns 403 on `dispatch` (this fixture is the non-staff
    // path), but crucially the POST body is RECORDED before the 403
    // is emitted — so we can still assert the wire shape via the
    // recorder even though the caller sees an error.
    #[test]
    fn dispatch_issue_body_carries_id_and_instance_id() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let err = dispatch_issue(&b, &t, "iss_42", "activ-2").unwrap_err();
        assert!(err.to_string().contains("403"), "non-staff ⇒ 403: {err}");

        let posts = m.posts.lock().unwrap();
        let disp = posts
            .iter()
            .find(|p| p.contains("\"action\":\"dispatch\""))
            .expect("dispatch body recorded");
        let v: serde_json::Value = serde_json::from_str(disp).unwrap();
        assert_eq!(v["action"], "dispatch");
        assert_eq!(v["id"], "iss_42");
        assert_eq!(
            v["instance_id"], "activ-2",
            "field name is `instance_id` (snake_case, per Manage contract)",
        );
    }

    // SDTEST-295 — Explicit coverage for the create_issue optional
    // field elision, which powers the Convert-to-Request bridge in
    // Support. `create_and_comment_bodies` above already asserts that
    // `source:"support"` reaches the wire; this test pins the
    // complementary edges:
    //   - source="" ⇒ the `source` key is OMITTED entirely (not sent
    //     as an empty string — the server defaults to "user" iff the
    //     key is absent).
    //   - source="support" ⇒ present AND value is a JSON string
    //     (not accidentally serialized as an object or number).
    #[test]
    fn create_issue_source_field_is_omitted_when_empty_and_present_when_support() {
        let m = start_mock();
        let (b, t) = cfg(&m);

        // Empty source ⇒ omitted from body.
        create_issue(&b, &t, "titre A", "", "", "").expect("empty source");
        // Explicit support source ⇒ present.
        create_issue(&b, &t, "titre B", "", "", "support").expect("source=support");

        let posts = m.posts.lock().unwrap();
        let a = serde_json::from_str::<serde_json::Value>(&posts[0]).unwrap();
        let b_ = serde_json::from_str::<serde_json::Value>(&posts[1]).unwrap();

        assert!(
            a.get("source").is_none(),
            "empty source must not be sent on the wire, got: {a}",
        );
        assert_eq!(
            b_.get("source").and_then(|v| v.as_str()),
            Some("support"),
            "source=support must land as a JSON string, got: {b_}",
        );
    }
}
