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
/// an ISO-8601 string (`lastAt`/`at` vary per channel). Result is epoch ms;
/// unparseable / null → `0.0`.
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

/// One message in a ticket thread. `from` is `"contact"` (customer),
/// `"note"` (internal), or an agent-side value; unknown `from` = agent-side.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SupportMessage {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub from: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub text: String,
    #[serde(default, deserialize_with = "de_flex_millis")]
    pub at: f64,
    #[serde(default)]
    pub name: Option<String>,
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
    #[serde(default, deserialize_with = "de_flex_millis")]
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
    /// A one-line channel glyph for the list.
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

/// Add an internal note (not sent to the customer).
pub fn support_note(base_url: &str, token: &str, id: &str, text: &str) -> Result<SupportTicket> {
    post_action(
        base_url,
        token,
        serde_json::json!({ "action": "note", "id": id, "text": text }),
    )
}

pub fn support_status(base_url: &str, token: &str, id: &str, status: &str) -> Result<SupportTicket> {
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
    fn channel_glyphs_have_a_fallback() {
        let mut t = SupportTicket::default();
        t.channel = "unknown".into();
        assert_eq!(t.channel_glyph(), "\u{2022}");
        t.channel = "livechat".into();
        assert_eq!(t.channel_glyph(), "\u{1F4AC}");
    }
}
