//! Hosted issue management (requests) — per tenant/site, GitHub-synced,
//! fleet-dispatchable. Users file requests; support/staff triage them.
//!
//! Endpoint: `{base}/api/manage/shelldeck/issues` (Bearer device token). Shapes
//! are snake_case with ISO-8601 timestamps (like the fleet API), parsed
//! defensively. Staff-only actions (status/assign/priority/dispatch/github)
//! return 403 for non-super-admin tokens.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;

// Keep one MiB of headroom for multipart framing below Bext's 10 MiB request cap.
pub const ISSUE_ATTACHMENT_MAX_BYTES: usize = 9 * 1024 * 1024;
pub const ISSUE_ATTACHMENT_MAX_COUNT: usize = 5;

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
    #[serde(default)]
    pub attachments: Vec<IssueAttachment>,
}

impl IssueComment {
    pub fn is_note(&self) -> bool {
        matches!(self.kind.as_str(), "status" | "system" | "github")
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IssueAttachment {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub share_id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub url: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub viewer_url: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub filename: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub content_type: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub created_by: String,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub created_at: f64,
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
    pub attachment_count: i64,
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
    pub attachments: Vec<IssueAttachment>,
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
    /// Whether the token is internal Support or super-admin staff — drives the triage action bar.
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

#[derive(Debug, Deserialize)]
struct AttachmentTicketResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    ticket: String,
    #[serde(default)]
    upload_url: String,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AttachmentUploadResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    receipt: String,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IssueAttachmentUpload {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl IssueAttachmentUpload {
    pub fn validate(&self) -> Result<()> {
        if !matches!(
            self.content_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        ) {
            return Err(ShellDeckError::Connection(
                "type image non pris en charge".to_string(),
            ));
        }
        if self.bytes.is_empty() || self.bytes.len() > ISSUE_ATTACHMENT_MAX_BYTES {
            return Err(ShellDeckError::Connection(
                "taille image invalide".to_string(),
            ));
        }
        if issue_image_content_type(&self.bytes) != Some(self.content_type.as_str()) {
            return Err(ShellDeckError::Connection(
                "contenu image invalide".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn issue_image_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Download an image URL with bounded redirects, time and size, then identify
/// it from its bytes rather than trusting its extension or Content-Type.
pub fn download_issue_image_url(url: &str) -> Result<IssueAttachmentUpload> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|_| ShellDeckError::Connection("URL image invalide".to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ShellDeckError::Connection(
            "URL HTTP(S) requise".to_string(),
        ));
    }
    let filename_hint = parsed
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("image")
        .to_string();
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))?;
    let response = client.get(parsed).send().map_err(|e| {
        ShellDeckError::Connection(format!("téléchargement image impossible : {}", e))
    })?;
    check_status(response.status().as_u16())?;
    if response
        .content_length()
        .is_some_and(|n| n > ISSUE_ATTACHMENT_MAX_BYTES as u64)
    {
        return Err(ShellDeckError::Connection(
            "L’image dépasse la limite de 9 Mo.".to_string(),
        ));
    }
    let mut bytes = Vec::new();
    response
        .take((ISSUE_ATTACHMENT_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| ShellDeckError::Connection(format!("lecture image impossible : {}", e)))?;
    if bytes.len() > ISSUE_ATTACHMENT_MAX_BYTES {
        return Err(ShellDeckError::Connection(
            "L’image dépasse la limite de 9 Mo.".to_string(),
        ));
    }
    let content_type = issue_image_content_type(&bytes).ok_or_else(|| {
        ShellDeckError::Connection("Format non pris en charge (PNG, JPEG ou WebP).".to_string())
    })?;
    let extension = match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "webp",
    };
    let filename = if filename_hint.contains('.') {
        filename_hint
    } else {
        format!("{filename_hint}.{extension}")
    };
    Ok(IssueAttachmentUpload {
        filename,
        content_type: content_type.to_string(),
        bytes,
    })
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

/// Server-side filter payload for `list_issues`. Any empty string / `None`
/// value is omitted from the query (server treats missing = "no filter").
/// The server ships the filters in two waves — `status`/`q` were the
/// original v1 pair, the rest ship in a bext PR (`feat/issues-soft-delete`);
/// the client can send them regardless because unknown query params are
/// silently ignored on the older server build.
#[derive(Debug, Clone, Default)]
pub struct IssueListFilter {
    pub status: String,
    pub q: String,
    pub priority: String,
    pub source: String,
    /// `""` = no filter, `"me"` / `"unassigned"` = special values, otherwise
    /// exact-match on `assignee` (email or name).
    pub assignee: String,
    /// Only requests filed by the caller (`requested_by === actor`).
    pub mine: bool,
    /// Staff-only tenant narrowing. Non-staff clients pass empty.
    pub tenant_id: String,
    /// `Some(true)` → only GitHub-linked, `Some(false)` → only non-linked,
    /// `None` → no filter.
    pub has_github: Option<bool>,
    /// ISO-8601 lower bound on `updated_at` (`""` = no bound).
    pub since: String,
}

/// Append `&key=percent_encoded(value)` to `query` when `value` is non-empty.
/// Skips the pair silently when the value is empty — the server treats a
/// missing filter as "no filter", so callers can push everything the user
/// might have populated.
fn push_query_kv(query: &mut String, key: &str, value: &str) {
    if !value.is_empty() {
        query.push_str(&format!(
            "&{key}={}",
            crate::config::cloud_account::percent_encode(value)
        ));
    }
}

/// List issues in the token's scope with the supplied filter. Empty fields
/// are omitted from the query string.
pub fn list_issues(base_url: &str, token: &str, filter: &IssueListFilter) -> Result<IssueList> {
    let mut query = String::from("?action=list");
    push_query_kv(&mut query, "status", &filter.status);
    push_query_kv(&mut query, "q", &filter.q);
    push_query_kv(&mut query, "priority", &filter.priority);
    push_query_kv(&mut query, "source", &filter.source);
    push_query_kv(&mut query, "assignee", &filter.assignee);
    if filter.mine {
        query.push_str("&mine=1");
    }
    push_query_kv(&mut query, "tenant_id", &filter.tenant_id);
    if let Some(gh) = filter.has_github {
        query.push_str(if gh { "&has_github=1" } else { "&has_github=0" });
    }
    push_query_kv(&mut query, "since", &filter.since);
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

/// Upload images to Inklura Share through short-lived issue-scoped tickets,
/// then return the receipts that Manage will validate while attaching them.
pub fn upload_issue_attachments(
    base_url: &str,
    token: &str,
    id: &str,
    uploads: &[IssueAttachmentUpload],
) -> Result<Vec<String>> {
    upload_scoped_attachments(&issues_url(base_url), token, id, uploads)
}

/// Shared ticket→Share upload handshake used by requests and Support tickets.
/// `ticket_url` remains a Manage endpoint; the long-lived sync token is never
/// sent to Share, which receives only the short-lived single-use capability.
pub fn upload_scoped_attachments(
    ticket_url: &str,
    token: &str,
    id: &str,
    uploads: &[IssueAttachmentUpload],
) -> Result<Vec<String>> {
    if uploads.len() > ISSUE_ATTACHMENT_MAX_COUNT {
        return Err(ShellDeckError::Connection(format!(
            "{} images maximum par envoi",
            ISSUE_ATTACHMENT_MAX_COUNT
        )));
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))?;
    let mut receipts = Vec::with_capacity(uploads.len());
    for upload in uploads {
        upload.validate()?;
        let ticket_resp = client
            .post(ticket_url)
            .bearer_auth(token)
            .json(&serde_json::json!({
                "action": "attachment-ticket", "id": id,
                "filename": upload.filename, "content_type": upload.content_type,
                "bytes": upload.bytes.len(),
            }))
            .send()
            .map_err(|e| ShellDeckError::Connection(format!("attachment ticket failed: {}", e)))?;
        check_status(ticket_resp.status().as_u16())?;
        let ticket: AttachmentTicketResponse = ticket_resp.json().map_err(|e| {
            ShellDeckError::Serialization(format!("invalid attachment ticket: {}", e))
        })?;
        if !ticket.ok || ticket.ticket.is_empty() || ticket.upload_url.is_empty() {
            return Err(ShellDeckError::Connection(
                ticket
                    .error
                    .unwrap_or_else(|| "ticket image refusé".to_string()),
            ));
        }

        let part = reqwest::blocking::multipart::Part::bytes(upload.bytes.clone())
            .file_name(upload.filename.clone())
            .mime_str(&upload.content_type)
            .map_err(|e| ShellDeckError::Connection(format!("invalid image type: {}", e)))?;
        let upload_resp = client
            .post(&ticket.upload_url)
            .bearer_auth(&ticket.ticket)
            .multipart(reqwest::blocking::multipart::Form::new().part("file", part))
            .send()
            .map_err(|e| ShellDeckError::Connection(format!("image upload failed: {}", e)))?;
        check_status(upload_resp.status().as_u16())?;
        let uploaded: AttachmentUploadResponse = upload_resp.json().map_err(|e| {
            ShellDeckError::Serialization(format!("invalid image upload response: {}", e))
        })?;
        if !uploaded.ok || uploaded.receipt.is_empty() {
            return Err(ShellDeckError::Connection(
                uploaded
                    .error
                    .unwrap_or_else(|| "upload image refusé".to_string()),
            ));
        }
        receipts.push(uploaded.receipt);
    }
    Ok(receipts)
}

/// Attach uploaded images to the main request body.
pub fn attach_issue_images(
    base_url: &str,
    token: &str,
    id: &str,
    receipts: &[String],
) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({
            "action": "attachment-attach", "id": id, "attachment_receipts": receipts,
        }),
    )
}

/// Add a text and/or image comment (mirrors both to GitHub when linked).
pub fn comment_issue_with_attachments(
    base_url: &str,
    token: &str,
    id: &str,
    body: &str,
    receipts: &[String],
) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({
            "action": "comment", "id": id, "body": body,
            "attachment_receipts": receipts,
        }),
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

/// Soft-delete a request (owner-or-staff — 403 otherwise). The server
/// stamps `deleted_at` and hides it from every read path; the row is
/// retained for audit. Returns the tombstoned issue.
pub fn delete_issue(base_url: &str, token: &str, id: &str) -> Result<Issue> {
    post_issue(
        base_url,
        token,
        serde_json::json!({ "action": "delete", "id": id }),
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
                        | "github-refresh" | "delete" => {
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
        "attachments":[{"id":"a1","share_id":"sh1","url":"https://share.inklura.fr/u/a1.png","viewer_url":"https://share.inklura.fr/s/a1","filename":"hero.png","content_type":"image/png","bytes":42,"created_by":"ben","created_at":"2026-07-02T20:04:00.000Z"}],
        "comments":[
          {"id":"c1","author":"ben","body":"des détails","kind":"comment","at":"2026-07-02T20:05:00.000Z","attachments":[{"id":"a2","url":"https://share.inklura.fr/u/a2.webp","filename":"detail.webp","content_type":"image/webp","bytes":84,"created_at":"2026-07-02T20:05:00.000Z"}]},
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
        let l = list_issues(&b, &t, &IssueListFilter::default()).expect("list");
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
        assert_eq!(iss.attachments.len(), 1);
        assert_eq!(iss.attachments[0].filename, "hero.png");
        assert_eq!(iss.comments[0].attachments.len(), 1);
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

    // SDTEST-1373 — Manage validates Share receipts server-side. The Rust
    // client must keep the receipt array in snake_case for both attachment
    // placement paths (request body and comment).
    #[test]
    fn attachment_receipt_bodies_match_manage_contract() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let receipts = vec!["receipt-a".to_string(), "receipt-b".to_string()];
        attach_issue_images(&b, &t, "iss_1", &receipts).expect("attach");
        comment_issue_with_attachments(&b, &t, "iss_1", "voir capture", &receipts)
            .expect("comment with images");

        let posts = m.posts.lock().unwrap();
        let attach: serde_json::Value = serde_json::from_str(
            posts
                .iter()
                .find(|p| p.contains("attachment-attach"))
                .expect("attach body"),
        )
        .unwrap();
        assert_eq!(attach["id"], "iss_1");
        assert_eq!(attach["attachment_receipts"][1], "receipt-b");
        let comment: serde_json::Value = serde_json::from_str(
            posts
                .iter()
                .find(|p| p.contains("voir capture"))
                .expect("comment body"),
        )
        .unwrap();
        assert_eq!(comment["attachment_receipts"][0], "receipt-a");
    }

    #[test]
    fn attachment_upload_rejects_spoofed_image_bytes() {
        let upload = IssueAttachmentUpload {
            filename: "fake.png".into(),
            content_type: "image/png".into(),
            bytes: b"nope".to_vec(),
        };
        assert!(upload.validate().is_err());
    }

    #[test]
    fn attachment_limit_keeps_multipart_below_bext_request_cap() {
        const BEXT_REQUEST_CAP: usize = 10 * 1024 * 1024;
        const MULTIPART_HEADROOM: usize = 1024 * 1024;
        assert!(ISSUE_ATTACHMENT_MAX_BYTES + MULTIPART_HEADROOM <= BEXT_REQUEST_CAP);
    }

    #[test]
    fn upload_issue_attachments_uses_ticket_and_multipart() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_thread = seen.clone();
        let upload_url = format!("{base}/upload");
        let handle = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let mut content_length = 0usize;
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                    headers.push_str(&line);
                }
                let mut body = vec![0u8; content_length];
                reader.read_exact(&mut body).unwrap();
                seen_thread.lock().unwrap().push(format!(
                    "{request}{headers}{}",
                    String::from_utf8_lossy(&body)
                ));
                let response_body = if index == 0 {
                    format!(
                        r#"{{"ok":true,"ticket":"{}","upload_url":"{}"}}"#,
                        "a".repeat(64),
                        upload_url
                    )
                } else {
                    r#"{"ok":true,"receipt":"receipt-1"}"#.to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(), response_body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&[0; 8]);
        let receipts = upload_issue_attachments(
            &base,
            TOKEN,
            "iss_1",
            &[IssueAttachmentUpload {
                filename: "capture.png".into(),
                content_type: "image/png".into(),
                bytes: png,
            }],
        )
        .expect("ticket + multipart upload");
        assert_eq!(receipts, vec!["receipt-1"]);
        handle.join().unwrap();
        let requests = seen.lock().unwrap();
        assert!(requests[0].contains("\"action\":\"attachment-ticket\""));
        assert!(requests[0].contains("\"bytes\":16"));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("content-type: multipart/form-data"));
        assert!(requests[1].contains("name=\"file\""));
        assert!(requests[1].contains("filename=\"capture.png\""));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {}", "a".repeat(64))));
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
        let err = list_issues(&m.url, "", &IssueListFilter::default()).unwrap_err();
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

    // SDTEST-301 — `delete_issue` is an owner-or-staff soft delete. The
    // server allows: staff (super-admin) for any in-scope request, or the
    // original filer (`requested_by` matches token identity). Non-owner
    // non-staff tokens surface 403 — the mock below models that path.
    // The POST body must carry exactly `{action:"delete", id}` — no stray
    // fields, and the action name is `delete` (a rename to "remove"/"destroy"
    // would silently 400 in prod). Deletion is never exercised live (it
    // would tombstone a real KV row), so the mock-recorded shape assertion
    // is the primary wire guard; the owner-allow path is a server-side
    // gate covered by the server's own integration tests.
    #[test]
    fn delete_issue_is_staff_gated_and_body_carries_action_and_id() {
        let m = start_mock();
        let (b, t) = cfg(&m);
        let err = delete_issue(&b, &t, "iss_42").unwrap_err();
        assert!(err.to_string().contains("403"), "non-staff ⇒ 403: {err}");

        let posts = m.posts.lock().unwrap();
        let del = posts
            .iter()
            .find(|p| p.contains("\"action\":\"delete\""))
            .expect("delete body recorded");
        let v: serde_json::Value = serde_json::from_str(del).unwrap();
        assert_eq!(v["action"], "delete");
        assert_eq!(v["id"], "iss_42");
        // no accidental extra keys beyond action + id
        assert_eq!(
            v.as_object().unwrap().len(),
            2,
            "delete body must be exactly {{action, id}}, got: {v}",
        );
    }

    // The 401 path also covers `delete` (auth checked before action).
    #[test]
    fn delete_issue_missing_bearer_surfaces_401() {
        let m = start_mock();
        let err = delete_issue(&m.url, "", "iss_1").unwrap_err();
        assert!(err.to_string().contains("401"), "got {}", err);
    }
}
