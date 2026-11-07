//! Inklura Manage notifications (the Manage bell) for the desktop app.
//!
//! `GET|POST {base}/api/manage/shelldeck/notifications`, Bearer device token.
//! The server merges the shared per-space feed of every site in the token's
//! directory (exports, agent alerts, automations…), masks prospect details the
//! account may not read, and never hands out raw Manage links: opening an item
//! asks for a short-lived signed switch URL (`open_url`) at click time.
//!
//! The personal staff inbox of the web bell needs a browser session and is not
//! part of this feed.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Severity reported by Manage (`success` / `error` / `info` / `warning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    Success,
    Error,
    Warning,
    #[default]
    #[serde(other)]
    Info,
}

/// One bell entry, already scoped to the site it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InkluraNotification {
    pub id: String,
    pub site_id: String,
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub site_label: String,
    #[serde(rename = "type", default)]
    pub kind: NotificationKind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Epoch milliseconds.
    #[serde(default)]
    pub ts: i64,
    #[serde(default)]
    pub read: bool,
    /// Whether opening the item lands on a specific Manage page (otherwise the
    /// space home).
    #[serde(default)]
    pub has_link: bool,
}

impl InkluraNotification {
    /// Identity across sites: feed ids are only unique inside one space.
    pub fn key(&self) -> (String, String) {
        (self.site_id.clone(), self.id.clone())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotificationFeed {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub unread: usize,
    #[serde(default)]
    pub items: Vec<InkluraNotification>,
    /// Number of spaces the server read for this account.
    #[serde(default)]
    pub sites: usize,
}

impl NotificationFeed {
    /// Apply a local read mark so the bell reacts before the next poll.
    pub fn mark_read_locally(&mut self, site_id: &str, id: &str) {
        if let Some(item) = self
            .items
            .iter_mut()
            .find(|n| n.site_id == site_id && n.id == id && !n.read)
        {
            item.read = true;
            self.unread = self.unread.saturating_sub(1);
        }
    }

    pub fn mark_all_read_locally(&mut self) {
        for item in &mut self.items {
            item.read = true;
        }
        self.unread = 0;
    }
}

/// Unread items that were not in the previously seen set, newest first.
/// `None` (first load) seeds silently: a launch must not replay old alerts.
pub fn fresh_unread<'a>(
    seen: Option<&HashSet<(String, String)>>,
    feed: &'a NotificationFeed,
) -> Vec<&'a InkluraNotification> {
    let Some(seen) = seen else {
        return Vec::new();
    };
    feed.items
        .iter()
        .filter(|n| !n.read && !seen.contains(&n.key()))
        .collect()
}

pub fn seen_keys(feed: &NotificationFeed) -> HashSet<(String, String)> {
    feed.items.iter().map(InkluraNotification::key).collect()
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

fn endpoint(base_url: &str) -> String {
    format!(
        "{}/api/manage/shelldeck/notifications",
        base_url.trim_end_matches('/')
    )
}

fn check(resp: reqwest::blocking::Response, what: &str) -> Result<reqwest::blocking::Response> {
    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(ShellDeckError::Connection(format!(
            "{} failed: HTTP {}",
            what,
            status.as_u16()
        )));
    }
    Ok(resp)
}

pub fn fetch_notifications(base_url: &str, token: &str) -> Result<NotificationFeed> {
    let resp = http_client()?
        .get(endpoint(base_url))
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("notifications request failed: {}", e)))?;
    check(resp, "notifications fetch")?
        .json::<NotificationFeed>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid notifications payload: {}", e)))
}

fn post(base_url: &str, token: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let resp = http_client()?
        .post(endpoint(base_url))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("notifications request failed: {}", e)))?;
    check(resp, "notifications update")?
        .json::<serde_json::Value>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid notifications payload: {}", e)))
}

pub fn mark_read(base_url: &str, token: &str, site_id: &str, id: &str) -> Result<()> {
    post(
        base_url,
        token,
        serde_json::json!({ "action": "markRead", "site_id": site_id, "id": id }),
    )
    .map(|_| ())
}

/// Mark every listed space read up to `before_ms` (the newest item the user saw),
/// so an alert that lands while the panel is open stays unread.
pub fn mark_all_read(base_url: &str, token: &str, before_ms: Option<i64>) -> Result<usize> {
    let mut body = serde_json::json!({ "action": "markAllRead" });
    if let Some(ms) = before_ms.and_then(chrono::DateTime::from_timestamp_millis) {
        body["before"] = serde_json::Value::String(ms.to_rfc3339());
    }
    let out = post(base_url, token, body)?;
    Ok(out.get("marked").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
}

/// A two-minute signed Manage URL that switches to the item's space and lands on
/// its page. Must be opened right away.
pub fn open_url(base_url: &str, token: &str, site_id: &str, id: Option<&str>) -> Result<String> {
    let mut body = serde_json::json!({ "action": "open", "site_id": site_id });
    if let Some(id) = id {
        body["id"] = serde_json::Value::String(id.to_string());
    }
    let out = post(base_url, token, body)?;
    match out.get("url").and_then(|v| v.as_str()) {
        Some(url) if url.starts_with("https://") || url.starts_with("http://127.0.0.1") => {
            Ok(url.to_string())
        }
        _ => Err(ShellDeckError::Serialization(
            "notification link missing from response".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const FEED: &str = r#"{"ok":true,"unread":2,"sites":2,"items":[
      {"id":"n_1","site_id":"s1","tenant_id":"t1","site_label":"Acme — Boutique","type":"success","title":"Export prêt","body":"","ts":1762500000000,"read":false,"has_link":true},
      {"id":"n_1","site_id":"s2","tenant_id":"t2","site_label":"Blog","type":"bogus","title":"Prospect","body":"Détails masqués","ts":1762400000000,"read":false,"has_link":false},
      {"id":"n_0","site_id":"s1","type":"warning","title":"Old","ts":1,"read":true}
    ]}"#;

    struct Seen {
        method: String,
        target: String,
        auth: String,
        body: String,
    }

    fn start_mock(status: u16, response: &'static str) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(8) {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let (mut auth, mut len) = (String::new(), 0usize);
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap_or(("", ""));
                    match k.trim().to_ascii_lowercase().as_str() {
                        "authorization" => auth = v.trim().to_string(),
                        "content-length" => len = v.trim().parse().unwrap_or(0),
                        _ => {}
                    }
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                let mut parts = line.split_whitespace();
                log.lock().unwrap().push(Seen {
                    method: parts.next().unwrap_or("").into(),
                    target: parts.next().unwrap_or("").into(),
                    auth,
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                    response.len()
                );
            }
        });
        (url, seen)
    }

    // SDTEST-2898
    #[test]
    fn feed_parses_per_site_items_and_unknown_kinds_fall_back_to_info() {
        let (url, seen) = start_mock(200, FEED);
        let feed = fetch_notifications(&format!("{url}/"), "sd_tok").unwrap();
        let req = &seen.lock().unwrap()[0];
        assert_eq!(req.method, "GET");
        assert_eq!(req.target, "/api/manage/shelldeck/notifications");
        assert_eq!(req.auth, "Bearer sd_tok");
        assert_eq!(feed.unread, 2);
        assert_eq!(feed.items[0].kind, NotificationKind::Success);
        assert_eq!(feed.items[1].kind, NotificationKind::Info);
        // Same feed id in two spaces stays two distinct notifications.
        assert_ne!(feed.items[0].key(), feed.items[1].key());
        assert_eq!(feed.items[2].site_label, "");
    }

    // SDTEST-2899
    #[test]
    fn revoked_token_is_classified_as_auth_rejected() {
        let (url, _) = start_mock(401, r#"{"ok":false}"#);
        let err = fetch_notifications(&url, "sd_old").unwrap_err();
        assert_eq!(
            crate::config::cloud_account::classify_api_error(&err),
            crate::config::cloud_account::ApiFailure::AuthRejected
        );
    }

    // SDTEST-2900
    #[test]
    fn updates_send_site_scoped_actions_and_a_cutoff() {
        let (url, seen) = start_mock(
            200,
            r#"{"ok":true,"marked":3,"url":"https://manage.inklura.fr/api/manage/switch?handoff=x"}"#,
        );
        mark_read(&url, "sd_tok", "s1", "n_1").unwrap();
        assert_eq!(
            mark_all_read(&url, "sd_tok", Some(1_762_500_000_000)).unwrap(),
            3
        );
        let link = open_url(&url, "sd_tok", "s2", Some("n_1")).unwrap();
        assert!(link.contains("/api/manage/switch?handoff="));
        let log = seen.lock().unwrap();
        let bodies: Vec<serde_json::Value> = log
            .iter()
            .map(|s| serde_json::from_str(&s.body).unwrap())
            .collect();
        assert!(log
            .iter()
            .all(|s| s.method == "POST" && s.auth == "Bearer sd_tok"));
        assert_eq!(
            bodies[0],
            serde_json::json!({"action":"markRead","site_id":"s1","id":"n_1"})
        );
        assert_eq!(bodies[1]["action"], "markAllRead");
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(bodies[1]["before"].as_str().unwrap())
                .unwrap()
                .timestamp_millis(),
            1_762_500_000_000
        );
        assert_eq!(
            bodies[2],
            serde_json::json!({"action":"open","site_id":"s2","id":"n_1"})
        );
    }

    // SDTEST-2901
    #[test]
    fn open_rejects_a_non_https_link() {
        let (url, _) = start_mock(200, r#"{"ok":true,"url":"javascript:alert(1)"}"#);
        assert!(open_url(&url, "sd_tok", "s1", None).is_err());
    }

    // SDTEST-2902
    #[test]
    fn first_load_seeds_silently_then_only_new_unread_items_surface() {
        let mut feed: NotificationFeed = serde_json::from_str(FEED).unwrap();
        assert!(fresh_unread(None, &feed).is_empty());
        let seen = seen_keys(&feed);
        feed.items.insert(
            0,
            InkluraNotification {
                id: "n_2".into(),
                site_id: "s1".into(),
                tenant_id: String::new(),
                site_label: String::new(),
                kind: NotificationKind::Info,
                title: "Nouveau".into(),
                body: String::new(),
                ts: 2,
                read: false,
                has_link: false,
            },
        );
        feed.unread = 3;
        let fresh = fresh_unread(Some(&seen), &feed);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, "n_2");
        feed.mark_read_locally("s1", "n_2");
        assert!(fresh_unread(Some(&seen), &feed).is_empty());
        assert_eq!(feed.unread, 2);
    }
}
