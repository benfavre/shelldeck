//! Support console client — the native support.inklura.fr helpdesk over the
//! token-gated `…/api/manage/shelldeck/support` endpoint.
//!
//! `GET  ?action=list`            → [`SupportListResponse`]
//! `GET  ?action=ticket&id=<id>`  → one [`SupportTicket`] with `messages`
//! `GET  ?action=agents`          → assignee picker
//! `POST {action, id, …}`         → an action, returns the updated ticket
//!
//! All shapes are parsed defensively (`#[serde(default)]` everywhere, unknown
//! fields ignored) so channel-specific quirks never break the console.

use crate::config::issues::{upload_scoped_attachments, IssueAttachment, IssueAttachmentUpload};
use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Deserialize a string field that the server may send as JSON `null`
/// (e.g. `message.from` on manage-channel tickets) — `null`/absent → `""`.
/// Plain `#[serde(default)]` only covers *absent*, not present-null.
fn de_nullable_string<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// Deserialize a timestamp the server sends as *either* an epoch-ms number or
/// an ISO-8601 string (`lastAt`/`at`/`createdAt` vary per channel). Result is
/// epoch ms; unparseable / null → `0.0`. Values in the seconds range are
/// scaled to milliseconds.
fn normalize_epoch_ms(n: f64) -> f64 {
    if n <= 0.0 || !n.is_finite() {
        return 0.0;
    }
    // Epoch seconds are ~1e9 today; epoch ms are ~1e12.
    if n < 1_000_000_000_000.0 {
        n * 1000.0
    } else {
        n
    }
}

fn parse_timestamp_str(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    if let Ok(n) = s.parse::<f64>() {
        return normalize_epoch_ms(n);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis() as f64;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3fZ") {
        return dt.timestamp_millis() as f64;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ") {
        return dt.timestamp_millis() as f64;
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return naive.and_utc().timestamp_millis() as f64;
        }
    }
    0.0
}

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
        Some(Flex::Num(n)) => normalize_epoch_ms(n),
        Some(Flex::Str(s)) => parse_timestamp_str(&s),
        None => 0.0,
    })
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SupportContact {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
}

impl SupportContact {
    /// Best display string for a contact.
    pub fn display(&self) -> String {
        self.name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.email.clone().filter(|s| !s.trim().is_empty()))
            .or_else(|| self.phone.clone().filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| "Contact".to_string())
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportSla {
    #[serde(default)]
    pub breaching: bool,
    #[serde(default)]
    pub breached: bool,
}

fn coalesce_timestamp(candidates: &[f64]) -> f64 {
    candidates.iter().copied().find(|t| *t > 0.0).unwrap_or(0.0)
}

fn de_support_message<'de, D>(deserializer: D) -> std::result::Result<SupportMessage, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize, Default)]
    #[allow(non_snake_case)] // Manage API wire format uses camelCase keys.
    struct Raw {
        #[serde(default, deserialize_with = "de_nullable_string")]
        from: String,
        #[serde(default, deserialize_with = "de_nullable_string")]
        text: String,
        #[serde(default, deserialize_with = "de_nullable_string")]
        dir: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        author: Option<String>,
        #[serde(default, deserialize_with = "de_flex_millis")]
        at: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        lastAt: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        createdAt: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        created_at: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        timestamp: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        updatedAt: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        sentAt: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        date: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        time: f64,
        #[serde(default, deserialize_with = "de_flex_millis")]
        ts: f64,
        #[serde(default)]
        attachments: Vec<IssueAttachment>,
    }
    let raw = Raw::deserialize(deserializer)?;
    let at = coalesce_timestamp(&[
        raw.at,
        raw.lastAt,
        raw.createdAt,
        raw.created_at,
        raw.timestamp,
        raw.updatedAt,
        raw.sentAt,
        raw.date,
        raw.time,
        raw.ts,
    ]);
    let from = if raw.from.is_empty() {
        match raw.dir.as_str() {
            "in" => "contact",
            "note" => "note",
            "out" => "agent",
            _ => "",
        }
        .to_string()
    } else {
        raw.from
    };
    Ok(SupportMessage {
        from,
        text: raw.text,
        at,
        name: raw.name.or(raw.author),
        attachments: raw.attachments,
    })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SupportMessage {
    pub from: String,
    pub text: String,
    pub at: f64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub attachments: Vec<IssueAttachment>,
}

impl<'de> Deserialize<'de> for SupportMessage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        de_support_message(deserializer)
    }
}

impl SupportMessage {
    pub fn is_customer(&self) -> bool {
        self.from.eq_ignore_ascii_case("contact")
    }
    pub fn is_note(&self) -> bool {
        self.from.eq_ignore_ascii_case("note")
    }
}

/// Slim ticket (list) — also carries `messages` when fetched by id.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportTicket {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub channel: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub subject: String,
    #[serde(default)]
    pub contact: SupportContact,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    #[serde(default)]
    pub unread: bool,
    /// Assignee email; empty string = unassigned.
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub assignee: String,
    /// Last-activity timestamp in ms (server may send an ISO string).
    #[serde(
        default,
        deserialize_with = "de_flex_millis",
        alias = "updatedAt",
        alias = "updated_at"
    )]
    pub last_at: f64,
    #[serde(default)]
    pub msg_count: u32,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub last_preview: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub resolution: String,
    #[serde(default)]
    pub reopened_count: u32,
    #[serde(default)]
    pub sla: SupportSla,
    /// Present only in the `?action=ticket` detail response.
    #[serde(default)]
    pub messages: Vec<SupportMessage>,
}

impl SupportTicket {
    pub fn is_unassigned(&self) -> bool {
        self.assignee.trim().is_empty()
    }
    /// A one-line channel glyph for the list (legacy emoji — prefer [`Self::channel_lucide`]).
    pub fn channel_glyph(&self) -> &'static str {
        match self.channel.as_str() {
            "livechat" => "\u{1F4AC}", // 💬
            "sms" => "\u{2709}",       // ✉
            "email" => "@",
            "contact" => "\u{270E}", // ✎
            "manage" => "\u{25C6}",  // ◆
            _ => "\u{2022}",         // •
        }
    }

    /// Lucide slug for the ticket channel (see `icons/lucide/` inventory).
    pub fn channel_lucide(&self) -> &'static str {
        match self.channel.as_str() {
            "livechat" => "reply",
            "email" => "mail",
            "sms" => "send",
            "contact" | "contactform" => "user",
            "manage" | "manage_area" => "server",
            _ => "inbox",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SupportCounts {
    #[serde(default)]
    pub all: u32,
    #[serde(default)]
    pub unassigned: u32,
    #[serde(default)]
    pub mine: u32,
    #[serde(default)]
    pub open: u32,
    #[serde(default)]
    pub pending: u32,
    #[serde(default)]
    pub breaching: u32,
    #[serde(default)]
    pub closed: u32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SupportMe {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub email: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default)]
    pub staff: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SupportAgent {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub email: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SupportListResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub tickets: Vec<SupportTicket>,
    #[serde(default)]
    pub counts: SupportCounts,
    #[serde(default)]
    pub me: SupportMe,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TicketResponse {
    #[serde(default)]
    ticket: SupportTicket,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AgentsResponse {
    #[serde(default)]
    agents: Vec<SupportAgent>,
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

fn support_url(base_url: &str) -> String {
    format!(
        "{}/api/manage/shelldeck/support",
        base_url.trim_end_matches('/')
    )
}

fn map_status(status: u16) -> Option<ShellDeckError> {
    match status {
        200..=299 => None,
        401 => Some(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        )),
        s => Some(ShellDeckError::Connection(format!(
            "support request failed: HTTP {}",
            s
        ))),
    }
}

/// Fetch the ticket list, counts, and the current agent identity.
pub fn support_list(base_url: &str, token: &str) -> Result<SupportListResponse> {
    let client = http_client()?;
    let resp = client
        .get(format!("{}?action=list", support_url(base_url)))
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("support list failed: {}", e)))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    resp.json::<SupportListResponse>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid support list: {}", e)))
}

/// Fetch a single ticket with its full message thread.
pub fn support_ticket(base_url: &str, token: &str, id: &str) -> Result<SupportTicket> {
    let client = http_client()?;
    let url = format!(
        "{}?action=ticket&id={}",
        support_url(base_url),
        crate::config::cloud_account::percent_encode(id)
    );
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("support ticket failed: {}", e)))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    let parsed: TicketResponse = resp
        .json()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid support ticket: {}", e)))?;
    Ok(parsed.ticket)
}

/// Fetch the assignee picker list.
pub fn support_agents(base_url: &str, token: &str) -> Result<Vec<SupportAgent>> {
    let client = http_client()?;
    let resp = client
        .get(format!("{}?action=agents", support_url(base_url)))
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("support agents failed: {}", e)))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    let parsed: AgentsResponse = resp
        .json()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid support agents: {}", e)))?;
    Ok(parsed.agents)
}

/// Shared POST helper: send an action body, return the updated ticket.
fn post_action(base_url: &str, token: &str, body: serde_json::Value) -> Result<SupportTicket> {
    let client = http_client()?;
    let resp = client
        .post(support_url(base_url))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("support action failed: {}", e)))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    let parsed: TicketResponse = resp
        .json()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid action response: {}", e)))?;
    Ok(parsed.ticket)
}

/// Reply to the customer on a ticket.
pub fn support_reply(base_url: &str, token: &str, id: &str, text: &str) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "reply", "id": id, "text": text }),
    )
}

/// Upload Support ticket images through the same scoped Share capability flow
/// as hosted requests.
pub fn upload_support_attachments(
    base_url: &str,
    token: &str,
    id: &str,
    uploads: &[IssueAttachmentUpload],
) -> Result<Vec<String>> {
    upload_scoped_attachments(&support_url(base_url), token, id, uploads)
}

pub fn support_reply_with_attachments(
    base_url: &str,
    token: &str,
    id: &str,
    text: &str,
    receipts: &[String],
) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({
            "action": "reply", "id": id, "text": text,
            "attachment_receipts": receipts,
        }),
    )
}

/// Add an internal note (not sent to the customer).
pub fn support_note(base_url: &str, token: &str, id: &str, text: &str) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "note", "id": id, "text": text }),
    )
}

pub fn support_note_with_attachments(
    base_url: &str,
    token: &str,
    id: &str,
    text: &str,
    receipts: &[String],
) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({
            "action": "note", "id": id, "text": text,
            "attachment_receipts": receipts,
        }),
    )
}

pub fn support_status(
    base_url: &str,
    token: &str,
    id: &str,
    status: &str,
) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "status", "id": id, "status": status }),
    )
}

pub fn support_priority(
    base_url: &str,
    token: &str,
    id: &str,
    priority: &str,
) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "priority", "id": id, "priority": priority }),
    )
}

/// Assign: `"me"`, an agent email, or `""` to unassign.
pub fn support_assign(
    base_url: &str,
    token: &str,
    id: &str,
    assignee: &str,
) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "assign", "id": id, "assignee": assignee }),
    )
}

pub fn support_resolve(
    base_url: &str,
    token: &str,
    id: &str,
    resolution: &str,
) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "resolve", "id": id, "resolution": resolution }),
    )
}

/// Mark a ticket read (clears the unread flag). Harmless.
pub fn support_read(base_url: &str, token: &str, id: &str) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "read", "id": id }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_FIXTURE: &str = r#"{
      "ok": true,
      "tickets": [
        { "id": "t1", "channel": "livechat", "subject": "Bonjour",
          "contact": { "name": "Alice", "email": "alice@ex.com" },
          "status": "open", "unread": true, "assignee": "",
          "lastAt": 1751470000000, "msgCount": 3, "lastPreview": "merci",
          "priority": "high", "tags": ["vip"], "resolution": "",
          "firstResponseDueAt": 0, "resolveDueAt": 0, "reopenedCount": 0,
          "csat": null, "sla": { "firstResponse": "ok", "resolve": "ok", "breaching": true, "breached": false } },
        { "id": "t2", "channel": "email", "subject": "Facture",
          "contact": { "email": "bob@ex.com" },
          "status": "pending", "unread": false, "assignee": "me@staff.com",
          "lastAt": 1751460000000, "msgCount": 1, "lastPreview": "?",
          "priority": "normal", "tags": [], "resolution": "",
          "sla": { "breaching": false, "breached": false } }
      ],
      "counts": { "all": 6, "unassigned": 2, "mine": 1, "open": 3, "pending": 2, "breaching": 1, "closed": 1 },
      "me": { "email": "me@staff.com", "name": "Me Staff", "staff": true }
    }"#;

    const TICKET_FIXTURE: &str = r#"{
      "ok": true,
      "ticket": {
        "id": "t1", "channel": "livechat", "subject": "Bonjour",
        "contact": { "name": "Alice" }, "status": "open", "assignee": "",
        "priority": "high", "sla": { "breaching": true, "breached": false },
        "messages": [
          { "from": "contact", "text": "Bonjour, un souci", "at": 1751460000000, "name": "Alice" },
          { "from": "agent", "text": "Bonjour Alice", "at": 1751461000000, "name": "Me" },
          { "from": "note", "text": "à escalader", "at": 1751462000000 },
          { "from": "weird_channel_value", "text": "auto", "at": 1751463000000 }
        ]
      }
    }"#;

    #[test]
    fn parse_list_fixture() {
        let r: SupportListResponse = serde_json::from_str(LIST_FIXTURE).expect("parse list");
        assert!(r.ok);
        assert_eq!(r.tickets.len(), 2);
        assert_eq!(r.counts.all, 6);
        assert_eq!(r.counts.unassigned, 2);
        assert_eq!(r.me.email, "me@staff.com");
        assert!(r.me.staff);

        let t1 = &r.tickets[0];
        assert_eq!(t1.id, "t1");
        assert_eq!(t1.channel, "livechat");
        assert!(t1.unread);
        assert!(t1.is_unassigned());
        assert!(t1.sla.breaching);
        assert_eq!(t1.priority, "high");
        assert_eq!(t1.contact.display(), "Alice");
        assert_eq!(t1.msg_count, 3);
        assert!((t1.last_at - 1751470000000.0).abs() < 1.0);

        let t2 = &r.tickets[1];
        assert!(!t2.is_unassigned());
        assert_eq!(t2.contact.display(), "bob@ex.com");
    }

    #[test]
    fn parse_ticket_fixture_classifies_messages() {
        let tr: TicketResponse = serde_json::from_str(TICKET_FIXTURE).expect("parse ticket");
        let t = tr.ticket;
        assert_eq!(t.messages.len(), 4);
        assert!(t.messages[0].is_customer());
        assert!(!t.messages[1].is_customer() && !t.messages[1].is_note());
        assert!(t.messages[2].is_note());
        // Unknown `from` is neither customer nor note → rendered agent-side.
        assert!(!t.messages[3].is_customer());
        assert!(!t.messages[3].is_note());
    }

    #[test]
    fn parses_null_message_and_ticket_strings() {
        // The live "manage" channel sends message.from = null and other string
        // fields as null; these must not break the parse.
        let json = r#"{
          "ticket": {
            "id": "x", "channel": null, "subject": null, "status": null,
            "assignee": null, "priority": null, "lastPreview": null, "resolution": null,
            "messages": [ { "from": null, "text": null, "at": 1751463000000 } ]
          }
        }"#;
        let tr: TicketResponse = serde_json::from_str(json).expect("parse nulls");
        let t = tr.ticket;
        assert_eq!(t.channel, "");
        assert_eq!(t.messages.len(), 1);
        assert_eq!(t.messages[0].from, "");
        assert_eq!(t.messages[0].text, "");
        // Null `from` is neither customer nor note → rendered agent-side.
        assert!(!t.messages[0].is_customer() && !t.messages[0].is_note());
    }

    #[test]
    fn parses_iso_string_and_numeric_timestamps() {
        // lastAt as an ISO-8601 string (live "email"/"contact" channels).
        let iso = r#"{"tickets":[
            {"id":"a","lastAt":"2026-06-30T09:39:24.631Z"},
            {"id":"b","lastAt":1782824571553},
            {"id":"c","lastAt":null}
        ],"counts":{},"me":{}}"#;
        let r: SupportListResponse = serde_json::from_str(iso).expect("parse mixed timestamps");
        assert!(r.tickets[0].last_at > 0.0, "ISO string should parse to ms");
        assert!((r.tickets[1].last_at - 1782824571553.0).abs() < 1.0);
        assert_eq!(r.tickets[2].last_at, 0.0);
    }

    #[test]
    fn parses_created_at_alias_and_epoch_seconds() {
        let ticket = r#"{"ticket":{
          "id":"t1",
          "lastAt":1751470000,
          "messages":[
            {"from":"contact","text":"hi","createdAt":"2026-06-30T09:39:24.631Z"},
            {"from":"agent","text":"yo","at":1751461000}
          ]
        }}"#;
        let tr: TicketResponse = serde_json::from_str(ticket).expect("parse aliases");
        assert!((tr.ticket.last_at - 1751470000000.0).abs() < 1.0);
        assert!(
            (tr.ticket.messages[0].at
                - chrono::DateTime::parse_from_rfc3339("2026-06-30T09:39:24.631Z")
                    .unwrap()
                    .timestamp_millis() as f64)
                .abs()
                < 1.0
        );
        assert!((tr.ticket.messages[1].at - 1751461000000.0).abs() < 1.0);
    }

    #[test]
    fn parses_message_last_at_alias() {
        let ticket = r#"{"ticket":{
          "id":"t1",
          "messages":[
            {"from":"contact","text":"hi","lastAt":"2026-06-30T09:39:24.631Z"},
            {"from":"agent","text":"yo","lastAt":1751461000}
          ]
        }}"#;
        let tr: TicketResponse = serde_json::from_str(ticket).expect("parse message lastAt");
        assert!(
            (tr.ticket.messages[0].at
                - chrono::DateTime::parse_from_rfc3339("2026-06-30T09:39:24.631Z")
                    .unwrap()
                    .timestamp_millis() as f64)
                .abs()
                < 1.0
        );
        assert!((tr.ticket.messages[1].at - 1751461000000.0).abs() < 1.0);
    }

    // SDTEST-1377
    #[test]
    fn support_messages_parse_share_attachments() {
        let ticket = r#"{"ticket":{"id":"t1","messages":[{
          "dir":"out","author":"Karim","text":"capture","ts":"2026-07-21T10:00:00.000Z",
          "attachments":[{"id":"abcdef123456","share_id":"abcdef123456",
            "url":"https://share.inklura.fr/u/abcdef123456.png",
            "viewer_url":"https://share.inklura.fr/s/abcdef123456",
            "filename":"capture.png","content_type":"image/png","bytes":42,
            "created_at":"2026-07-21T10:00:00.000Z"}]
        }]}}"#;
        let parsed: TicketResponse = serde_json::from_str(ticket).expect("support attachment");
        let attachment = &parsed.ticket.messages[0].attachments[0];
        assert_eq!(attachment.filename, "capture.png");
        assert_eq!(
            attachment.viewer_url,
            "https://share.inklura.fr/s/abcdef123456"
        );
        assert_eq!(parsed.ticket.messages[0].from, "agent");
        assert_eq!(parsed.ticket.messages[0].name.as_deref(), Some("Karim"));
    }

    #[test]
    fn channel_glyphs_have_a_fallback() {
        let mut t = SupportTicket::default();
        t.channel = "unknown".into();
        assert_eq!(t.channel_glyph(), "\u{2022}");
        t.channel = "livechat".into();
        assert_eq!(t.channel_glyph(), "\u{1F4AC}");
    }

    #[test]
    fn channel_lucide_maps_known_channels() {
        let mut t = SupportTicket::default();
        t.channel = "email".into();
        assert_eq!(t.channel_lucide(), "mail");
        t.channel = "livechat".into();
        assert_eq!(t.channel_lucide(), "reply");
        t.channel = "unknown".into();
        assert_eq!(t.channel_lucide(), "inbox");
    }

    // ── SDTEST-225 — Body shapes for the 7 support write endpoints ─────
    //
    // Every write goes through `post_action` (POST to
    // `/api/manage/shelldeck/support` with a JSON body carrying an
    // `action` discriminator + endpoint-specific fields). A drift here
    // (renamed field, missing action key, wrong JSON type) silently
    // 400s on production Manage — the test surface catches it before
    // the toast does.
    //
    // Mock pattern matches `issues.rs::start_mock` and
    // `jean_fleet.rs::start_mock` — zero-dep loopback TcpListener
    // + std thread, records every POST body verbatim, echoes back a
    // canonical ticket response so `post_action` can parse Ok.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct SupportMock {
        url: String,
        posts: Arc<Mutex<Vec<String>>>,
        _handle: std::thread::JoinHandle<()>,
    }

    /// Canonical response body: minimal `TicketResponse`-shaped payload.
    const TICKET_ECHO: &str = r#"{
      "ok": true,
      "ticket": {
        "id": "t_echo", "channel": "email", "subject": "s",
        "status": "open", "assignee": "", "priority": "normal",
        "messages": []
      }
    }"#;

    fn start_support_write_mock() -> SupportMock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let posts = Arc::new(Mutex::new(Vec::<String>::new()));
        let posts2 = posts.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..16 {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let mut auth_ok = false;
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
                        if k == "authorization" && v.starts_with("Bearer ") {
                            auth_ok = true;
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

                let (status_line, out): (&str, String) = if !auth_ok {
                    ("401 Unauthorized", r#"{"ok":false}"#.into())
                } else if method == "POST" && target.contains("/api/manage/shelldeck/support") {
                    posts2.lock().unwrap().push(body);
                    ("200 OK", TICKET_ECHO.into())
                } else {
                    ("404 Not Found", r#"{"ok":false}"#.into())
                };
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    out.as_bytes().len(),
                    out,
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        SupportMock {
            url: format!("http://127.0.0.1:{}", port),
            posts,
            _handle: handle,
        }
    }

    const WRITE_TOKEN: &str = "sd_write_test";
    const TICKET_ID: &str = "t_test_1";

    /// Parse one recorded POST body into a JSON value for assertions.
    fn recorded(mock: &SupportMock, i: usize) -> serde_json::Value {
        let posts = mock.posts.lock().unwrap();
        serde_json::from_str(&posts[i]).expect("body is valid JSON")
    }

    #[test]
    fn support_reply_sends_reply_action_with_text() {
        let m = start_support_write_mock();
        super::support_reply(&m.url, WRITE_TOKEN, TICKET_ID, "Bonjour Alice")
            .expect("reply mock returns TICKET_ECHO");
        let body = recorded(&m, 0);
        assert_eq!(body["action"], "reply");
        assert_eq!(body["id"], TICKET_ID);
        assert_eq!(body["text"], "Bonjour Alice");
    }

    #[test]
    fn support_note_sends_note_action_with_text() {
        let m = start_support_write_mock();
        super::support_note(&m.url, WRITE_TOKEN, TICKET_ID, "à escalader")
            .expect("note mock returns TICKET_ECHO");
        let body = recorded(&m, 0);
        assert_eq!(body["action"], "note");
        assert_eq!(body["id"], TICKET_ID);
        assert_eq!(body["text"], "à escalader");
    }

    // SDTEST-1377
    #[test]
    fn support_reply_and_note_send_attachment_receipts() {
        let m = start_support_write_mock();
        let receipts = vec!["receipt-a".to_string(), "receipt-b".to_string()];
        super::support_reply_with_attachments(
            &m.url,
            WRITE_TOKEN,
            TICKET_ID,
            "Voir captures",
            &receipts,
        )
        .expect("reply with attachments");
        super::support_note_with_attachments(&m.url, WRITE_TOKEN, TICKET_ID, "", &receipts)
            .expect("note with attachments");

        let reply = recorded(&m, 0);
        let note = recorded(&m, 1);
        assert_eq!(reply["attachment_receipts"][0], "receipt-a");
        assert_eq!(reply["attachment_receipts"][1], "receipt-b");
        assert_eq!(note["action"], "note");
        assert_eq!(note["attachment_receipts"][0], "receipt-a");
    }

    #[test]
    fn support_status_sends_status_action_with_status_field() {
        let m = start_support_write_mock();
        super::support_status(&m.url, WRITE_TOKEN, TICKET_ID, "in_progress")
            .expect("status mock returns TICKET_ECHO");
        let body = recorded(&m, 0);
        assert_eq!(body["action"], "status");
        assert_eq!(body["id"], TICKET_ID);
        assert_eq!(body["status"], "in_progress");
    }

    #[test]
    fn support_priority_sends_priority_action_with_priority_field() {
        let m = start_support_write_mock();
        super::support_priority(&m.url, WRITE_TOKEN, TICKET_ID, "urgent")
            .expect("priority mock returns TICKET_ECHO");
        let body = recorded(&m, 0);
        assert_eq!(body["action"], "priority");
        assert_eq!(body["id"], TICKET_ID);
        assert_eq!(body["priority"], "urgent");
    }

    #[test]
    fn support_assign_sends_assign_action_with_assignee_field() {
        let m = start_support_write_mock();
        // "me" — the workspace shortcut for self-assign.
        super::support_assign(&m.url, WRITE_TOKEN, TICKET_ID, "me")
            .expect("assign mock returns TICKET_ECHO");
        // Empty string — the workspace shortcut for unassign.
        super::support_assign(&m.url, WRITE_TOKEN, TICKET_ID, "")
            .expect("unassign mock returns TICKET_ECHO");

        let b0 = recorded(&m, 0);
        assert_eq!(b0["action"], "assign");
        assert_eq!(b0["assignee"], "me");
        let b1 = recorded(&m, 1);
        assert_eq!(b1["action"], "assign");
        assert_eq!(b1["assignee"], "");
    }

    #[test]
    fn support_resolve_sends_resolve_action_with_resolution_field() {
        let m = start_support_write_mock();
        super::support_resolve(&m.url, WRITE_TOKEN, TICKET_ID, "duplicate")
            .expect("resolve mock returns TICKET_ECHO");
        let body = recorded(&m, 0);
        assert_eq!(body["action"], "resolve");
        assert_eq!(body["id"], TICKET_ID);
        assert_eq!(body["resolution"], "duplicate");
    }

    #[test]
    fn support_read_sends_read_action_id_only() {
        let m = start_support_write_mock();
        super::support_read(&m.url, WRITE_TOKEN, TICKET_ID).expect("read mock returns TICKET_ECHO");
        let body = recorded(&m, 0);
        assert_eq!(body["action"], "read");
        assert_eq!(body["id"], TICKET_ID);
        // No extra payload — mark-read is a bare acknowledgment.
        assert!(body["text"].is_null());
        assert!(body["status"].is_null());
    }

    #[test]
    fn support_writes_surface_401_when_bearer_missing() {
        // Belt-and-suspenders: a request with an empty token (which the
        // mock treats as missing Bearer) surfaces a typed error, so a
        // rejected session doesn't silently no-op.
        let m = start_support_write_mock();
        let err =
            super::support_reply(&m.url, "", TICKET_ID, "hi").expect_err("no bearer must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("401") || msg.to_lowercase().contains("rejected"),
            "expected 401/rejected in error, got: {msg}",
        );
    }

    /// One-shot canned GET responder — serves a fixed JSON body to the
    /// next N accepted requests. Used for the P2 defensive tests below
    /// where we only need to verify the parser's behaviour on a
    /// specific server payload, not any request-side assertion.
    fn spawn_canned_get(
        status_line: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut l = String::new();
                    if reader.read_line(&mut l).unwrap_or(0) == 0 {
                        break;
                    }
                    if l.trim_end().is_empty() {
                        break;
                    }
                }
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body,
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{}", port), handle)
    }

    // SDTEST-227 — `support_agents` on an empty list must return `Ok(vec![])`
    // without panicking. Fresh tenants have zero staff members; the picker
    // then renders "aucun agent" instead of crashing.
    #[test]
    fn support_agents_returns_empty_vec_cleanly() {
        let (url, _h) = spawn_canned_get("200 OK", r#"{"ok":true,"agents":[]}"#);
        let agents = super::support_agents(&url, WRITE_TOKEN).expect("empty agents parses");
        assert!(agents.is_empty());
    }

    // SDTEST-228 — `support_list` preserves the server's order.
    // Manage sorts by `lastAt` desc server-side; ShellDeck must NOT
    // resort (a client-side re-order would drop unread/breaching
    // tickets from the top of the list). This pins the pass-through.
    #[test]
    fn support_list_preserves_server_order() {
        // Deliberately anti-sorted alphabetically to catch an accidental
        // sort_by(|t| t.id) refactor.
        let body = r#"{
          "ok": true, "staff": true,
          "tickets": [
            { "id":"z", "channel":"email", "subject":"z", "status":"open", "priority":"low",  "assignee":"", "lastAt":3, "unread":false, "messages":[] },
            { "id":"a", "channel":"email", "subject":"a", "status":"open", "priority":"high", "assignee":"", "lastAt":2, "unread":false, "messages":[] },
            { "id":"m", "channel":"email", "subject":"m", "status":"open", "priority":"normal", "assignee":"", "lastAt":1, "unread":false, "messages":[] }
          ],
          "counts": {"all":3,"unassigned":3,"mine":0,"open":3,"pending":0,"breaching":0,"closed":0},
          "me": {"email":"me@x","name":"Me","staff":true}
        }"#;
        // We need a *runtime* string so the whole test isn't tied to the
        // 'static requirement of the canned helper — use include_str-style
        // static reference via a Box::leak (test-only).
        let leaked: &'static str = Box::leak(body.to_string().into_boxed_str());
        let (url, _h) = spawn_canned_get("200 OK", leaked);

        let r = super::support_list(&url, WRITE_TOKEN).expect("parse list");
        let ids: Vec<&str> = r.tickets.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["z", "a", "m"],
            "server order MUST be preserved, no client-side sort",
        );
    }
}
