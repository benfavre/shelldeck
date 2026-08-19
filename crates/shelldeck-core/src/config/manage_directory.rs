//! Mentionable people, from Inklura Manage.
//!
//! The composer's `@` picker may only offer a person the caller is entitled to
//! address. That verdict needs role information, which no existing ShellDeck
//! route carries: `…/support?action=agents` is staff-only *and* returns
//! `{name, email}` with no roles, so it can neither exclude super-admins nor
//! identify support agents. Building people mentions on it would break the
//! first rule in `docs/ai-mentions.md` § 5.3.
//!
//! This module therefore talks to a dedicated directory endpoint:
//!
//! ```text
//! GET /api/manage/shelldeck/directory?action=people[&site_id=<uuid>]
//! Authorization: Bearer <sync token>
//! → 200 { ok, people: [{ id, name, email, roles[], site_id, relation, mentionable }] }
//! ```
//!
//! **That route ships in the `bext` repo as its own PR** (same precedent as the
//! issues soft-delete in `AGENTS.md`). Until it lands, Manage answers 404 and
//! [`fetch_people`] returns an empty directory rather than an error: every
//! other mention kind keeps working and no "Personnes" section is shown.
//!
//! The client never trusts the server's verdict alone — `mentions::
//! person_is_mentionable` re-checks the role bag, because one flag from one
//! server is one deploy bug away from leaking platform-staff identities into a
//! customer-facing picker.

use crate::ai::mentions::person_is_mentionable;
use crate::config::cloud_account::percent_encode;
use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One person Manage considers addressable for the calling token.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryPerson {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    /// CM role bag, already filtered server-side. Re-checked client-side.
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub site_id: Option<String>,
    #[serde(default)]
    pub site_label: Option<String>,
    /// `support_agent` | `member`, mapped to `mentions::PersonRelation`.
    #[serde(default)]
    pub relation: String,
    /// The server's own verdict. Authoritative when `false`.
    #[serde(default)]
    pub mentionable: Option<bool>,
}

impl DirectoryPerson {
    /// Best display label; never the empty string.
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.trim().to_string()
        } else if !self.email.trim().is_empty() {
            self.email.trim().to_string()
        } else {
            self.id.clone()
        }
    }

    pub fn is_support_agent(&self) -> bool {
        self.relation == "support_agent"
            || self
                .roles
                .iter()
                .any(|role| role.trim().eq_ignore_ascii_case("inklura_support"))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PeopleResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    people: Vec<DirectoryPerson>,
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

/// Fetch the people the caller may mention, optionally scoped to one site.
///
/// Returns an empty list — not an error — when the endpoint is absent (404) or
/// rejects the requested scope (403). Both mean "no people to offer here", and
/// neither is worth a toast in a picker the user just opened.
pub fn fetch_people(
    base_url: &str,
    token: &str,
    site_id: Option<&str>,
) -> Result<Vec<DirectoryPerson>> {
    let mut url = format!(
        "{}/api/manage/shelldeck/directory?action=people",
        base_url.trim_end_matches('/')
    );
    if let Some(site) = site_id.filter(|value| !value.trim().is_empty()) {
        url.push_str(&format!("&site_id={}", percent_encode(site)));
    }
    let response = http_client()?
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("directory request failed: {}", e)))?;

    let status = response.status();
    if status.as_u16() == 401 {
        return Err(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        ));
    }
    // 404 = the route has not shipped yet; 403/400 = this token owns no such
    // scope. Neither is an error the user can act on.
    if matches!(status.as_u16(), 400 | 403 | 404 | 405) {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        return Err(ShellDeckError::Connection(format!(
            "directory fetch failed: HTTP {}",
            status.as_u16()
        )));
    }
    let payload = response
        .json::<PeopleResponse>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid directory payload: {}", e)))?;
    if !payload.ok {
        return Ok(Vec::new());
    }
    Ok(filter_mentionable(payload.people))
}

/// Drop everyone the composer must never offer, whatever the server said.
pub fn filter_mentionable(people: Vec<DirectoryPerson>) -> Vec<DirectoryPerson> {
    people
        .into_iter()
        .filter(|person| !person.email.trim().is_empty())
        .filter(|person| person_is_mentionable(&person.roles, person.mentionable))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Serve one request and hand back what was asked for.
    fn serve(
        body: &'static str,
        status: &'static str,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(&stream);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).expect("request line");
            let mut authorization = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                if line.to_ascii_lowercase().starts_with("authorization:") {
                    authorization = line.trim().to_string();
                }
            }
            let mut stream = &stream;
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.flush();
            format!("{} | {}", request_line.trim(), authorization)
        });
        (base, handle)
    }

    // SDTEST-1648
    #[test]
    fn people_request_carries_the_bearer_token_and_the_site_scope() {
        let (base, handle) = serve(r#"{"ok":true,"people":[]}"#, "200 OK");
        let people = fetch_people(&base, "sd_token", Some("site-a")).expect("fetch");
        assert!(people.is_empty());
        let observed = handle.join().expect("server thread");
        assert!(observed.contains("action=people"));
        assert!(observed.contains("site_id=site-a"));
        assert!(observed.contains("Bearer sd_token"));
    }

    // SDTEST-1649
    #[test]
    fn a_missing_endpoint_is_an_empty_directory_not_a_failure() {
        for status in ["404 Not Found", "403 Forbidden", "400 Bad Request"] {
            let (base, handle) = serve(r#"{"ok":false}"#, status);
            let people = fetch_people(&base, "sd_token", None).expect("degrade quietly");
            assert!(people.is_empty(), "{status} should degrade to empty");
            let _ = handle.join();
        }
    }

    // SDTEST-1650
    #[test]
    fn an_expired_token_is_reported_so_the_session_can_be_invalidated() {
        let (base, handle) = serve(r#"{"ok":false}"#, "401 Unauthorized");
        let error = fetch_people(&base, "stale", None).unwrap_err();
        assert!(error.to_string().contains("401"));
        let _ = handle.join();
    }

    // SDTEST-1651
    #[test]
    fn super_admins_are_dropped_even_when_the_server_marks_them_mentionable() {
        let (base, handle) = serve(
            r#"{"ok":true,"people":[
                {"id":"1","name":"Agent","email":"agent@inklura.fr","roles":["inklura_support"],"relation":"support_agent","mentionable":true},
                {"id":"2","name":"Root","email":"root@inklura.fr","roles":["superadmin"],"relation":"member","mentionable":true},
                {"id":"3","name":"Muted","email":"muted@client.fr","roles":[],"relation":"member","mentionable":false},
                {"id":"4","name":"Sans mail","email":"","roles":[],"relation":"member","mentionable":true}
            ]}"#,
            "200 OK",
        );
        let people = fetch_people(&base, "sd_token", None).expect("fetch");
        let _ = handle.join();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].email, "agent@inklura.fr");
        assert!(people[0].is_support_agent());
    }
}
