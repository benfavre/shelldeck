//! Inklura Manage sites directory + area deep links.
//!
//! [`fetch_sites`] pulls the tenant's site list and the set of manage areas
//! (CMS, helpdesk, e-commerce, …) the portal exposes. [`manage_area_url`]
//! builds a browser deep link that switches the manage scope to a given site
//! and lands on an area — `open_in_browser` handles the rest.

use crate::config::cloud_account::percent_encode;
use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One manageable site (tenant × site) from the portal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedSiteInfo {
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub site_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tenant_name: String,
    /// Public host for the site; may be empty.
    #[serde(default)]
    pub host: String,
    /// Combined "Tenant — Site" label for the picker.
    #[serde(default)]
    pub label: String,
    /// Optional metadata sourced from `sitemeta:<tenant_id>` server-side.
    /// Populated when the manage console has probed the site; `None` for
    /// unprobed sites (User mode falls back to a generic card).
    #[serde(default)]
    pub favicon: Option<String>,
    /// Brand accent as `#rrggbb`.
    #[serde(default)]
    pub brand_color: Option<String>,
    #[serde(default)]
    pub is_wordpress: Option<bool>,
    /// Absolute wp-admin URL when the site is WordPress; enables the
    /// "Ouvrir wp-admin" chip on User-mode site cards.
    #[serde(default)]
    pub wp_admin_url: Option<String>,
}

impl ManagedSiteInfo {
    /// Best display label, falling back to name/host when `label` is empty.
    pub fn display_label(&self) -> String {
        if !self.label.trim().is_empty() {
            self.label.clone()
        } else if !self.name.trim().is_empty() {
            self.name.clone()
        } else if !self.host.trim().is_empty() {
            self.host.clone()
        } else {
            self.site_id.clone()
        }
    }
}

/// A manage area (deep-link target), e.g. `{ key: "cms", label: "Contenu (CMS)", path: "/manage/cms" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManageArea {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub path: String,
}

/// Response from `GET …/sites`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SitesPayload {
    #[serde(default)]
    pub ok: bool,
    /// Origin to build deep links against (e.g. `https://manage.inklura.fr`).
    #[serde(default)]
    pub manage_origin: String,
    #[serde(default)]
    pub sites: Vec<ManagedSiteInfo>,
    #[serde(default)]
    pub areas: Vec<ManageArea>,
    /// Monique dashboard config, delivered only to super-admin devices.
    #[serde(default)]
    pub monique: Option<crate::config::monique::MoniqueConfig>,
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

/// Fetch the sites directory + manage areas for the signed-in account.
pub fn fetch_sites(base_url: &str, token: &str) -> Result<SitesPayload> {
    let url = format!(
        "{}/api/manage/shelldeck/sites",
        base_url.trim_end_matches('/')
    );
    let client = http_client()?;
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("sites request failed: {}", e)))?;

    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(ShellDeckError::Connection(format!(
            "sites fetch failed: HTTP {}",
            status.as_u16()
        )));
    }
    resp.json::<SitesPayload>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid sites payload: {}", e)))
}

/// Mint a fresh signed Boat handoff at click time. Cached directory links expire;
/// the legacy unsigned scope-switch URL is not accepted by current Manage hosts.
pub fn fetch_boat_console_url(
    base_url: &str,
    token: &str,
    tenant_id: &str,
    site_id: &str,
) -> Result<String> {
    let base = reqwest::Url::parse(base_url)
        .map_err(|_| ShellDeckError::Connection("invalid Manage origin".into()))?;
    let loopback = matches!(base.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
    if (base.scheme() != "https" && !(base.scheme() == "http" && loopback))
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(ShellDeckError::Connection("invalid Manage origin".into()));
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ShellDeckError::Connection("failed to build Manage client".into()))?;
    let response = client
        .get(format!(
            "{}/api/manage/shelldeck/sites",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .send()
        .map_err(|_| ShellDeckError::Connection("Boat handoff request failed".into()))?;
    if !response.status().is_success() {
        return Err(ShellDeckError::Connection(
            "Boat handoff was refused".into(),
        ));
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(response, 2 * 1024 * 1024 + 1),
        &mut bytes,
    )
    .map_err(|_| ShellDeckError::Connection("Boat directory read failed".into()))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(ShellDeckError::Serialization(
            "Boat directory is too large".into(),
        ));
    }
    let directory: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| ShellDeckError::Serialization("invalid Boat handoff directory".into()))?;
    let target = directory["sites"]
        .as_array()
        .and_then(|sites| {
            sites
                .iter()
                .find(|site| site["tenant_id"] == tenant_id && site["site_id"] == site_id)
        })
        .and_then(|site| site["switch_urls"]["boat"].as_str())
        .ok_or_else(|| {
            ShellDeckError::Connection("Boat console is unavailable for this site".into())
        })?;
    let url = reqwest::Url::parse(target)
        .map_err(|_| ShellDeckError::Connection("invalid Boat handoff".into()))?;
    if url.origin() != base.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() != "/api/manage/switch"
        || !url
            .query_pairs()
            .any(|(key, value)| key == "handoff" && !value.is_empty())
        || url.query_pairs().any(|(key, _)| key != "handoff")
    {
        return Err(ShellDeckError::Connection("invalid Boat handoff".into()));
    }
    Ok(url.to_string())
}

/// Build the browser deep link that switches the manage scope to `site` and
/// lands on `area_path`:
/// `{manage_origin}/api/manage/switch?tenantId=&siteId=&host=&label=&next=`.
pub fn manage_area_url(manage_origin: &str, site: &ManagedSiteInfo, area_path: &str) -> String {
    format!(
        "{}/api/manage/switch?tenantId={}&siteId={}&host={}&label={}&next={}",
        manage_origin.trim_end_matches('/'),
        percent_encode(&site.tenant_id),
        percent_encode(&site.site_id),
        percent_encode(&site.host),
        percent_encode(&site.label),
        percent_encode(area_path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2914 / SDUC-571: the server chooses the scope and signs it; no unsigned fallback.
    #[test]
    fn sdtest_2914_boat_handoff_is_fresh_scoped_and_same_origin() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        for variant in ["valid", "other-site", "other-origin", "unsigned", "missing"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let target = match variant {
                "other-origin" => {
                    "https://untrusted.example/api/manage/switch?handoff=secret".to_string()
                }
                "unsigned" => format!("{origin}/api/manage/switch?tenantId=t&siteId=s"),
                _ => format!("{origin}/api/manage/switch?handoff=fresh-signature"),
            };
            let body = serde_json::json!({"sites": [{
                "tenant_id": "t", "site_id": if variant == "other-site" { "other" } else { "s" },
                "switch_urls": if variant == "missing" { serde_json::json!({}) } else { serde_json::json!({"boat": target}) }
            }]}).to_string();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("GET /api/manage/shelldeck/sites HTTP/1.1"));
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer sd_test"));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            });
            let result = fetch_boat_console_url(&origin, "sd_test", "t", "s");
            server.join().unwrap();
            if variant == "valid" {
                assert_eq!(
                    result.unwrap(),
                    format!("{origin}/api/manage/switch?handoff=fresh-signature")
                );
            } else {
                assert!(result.is_err(), "accepted {variant}");
            }
        }
    }

    fn site() -> ManagedSiteInfo {
        ManagedSiteInfo {
            tenant_id: "t-123".to_string(),
            site_id: "s-456".to_string(),
            name: "Dashboard".to_string(),
            tenant_name: "Inklura".to_string(),
            host: "dashboard.inklura.fr".to_string(),
            label: "Inklura — Dashboard".to_string(),
            favicon: None,
            brand_color: None,
            is_wordpress: None,
            wp_admin_url: None,
        }
    }

    #[test]
    fn area_url_encodes_all_params() {
        let url = manage_area_url("https://manage.inklura.fr/", &site(), "/manage/cms");
        assert_eq!(
            url,
            "https://manage.inklura.fr/api/manage/switch?tenantId=t-123&siteId=s-456\
             &host=dashboard.inklura.fr&label=Inklura%20%E2%80%94%20Dashboard&next=%2Fmanage%2Fcms"
        );
    }

    #[test]
    fn area_url_handles_empty_host() {
        let mut s = site();
        s.host = String::new();
        let url = manage_area_url("https://manage.inklura.fr", &s, "/manage/helpdesk");
        assert!(url.contains("host=&"));
        assert!(url.ends_with("next=%2Fmanage%2Fhelpdesk"));
    }

    #[test]
    fn sites_payload_parses_contract_example() {
        let json = r#"{
          "ok": true,
          "manage_origin": "https://manage.inklura.fr",
          "sites": [
            { "tenant_id": "t1", "site_id": "s1", "name": "Site A", "tenant_name": "Acme",
              "host": "a.example.com", "label": "Acme — Site A" },
            { "tenant_id": "t2", "site_id": "s2", "name": "Site B", "tenant_name": "Beta",
              "host": "", "label": "Beta — Site B" }
          ],
          "areas": [
            { "key": "cms", "label": "Contenu (CMS)", "path": "/manage/cms" },
            { "key": "helpdesk", "label": "Support", "path": "/manage/helpdesk" }
          ],
          "monique": {
            "url": "https://monique.1clic.pro",
            "user": "ops",
            "pass": "secret"
          }
        }"#;
        let p: SitesPayload = serde_json::from_str(json).expect("parse sites");
        assert!(p.ok);
        assert_eq!(p.manage_origin, "https://manage.inklura.fr");
        assert_eq!(p.sites.len(), 2);
        assert_eq!(p.sites[1].host, "");
        assert_eq!(p.sites[1].display_label(), "Beta — Site B");
        assert_eq!(p.areas.len(), 2);
        assert_eq!(p.areas[0].key, "cms");
        assert_eq!(p.areas[0].path, "/manage/cms");
        let monique = p.monique.expect("Monique config");
        assert_eq!(monique.url, "https://monique.1clic.pro");
        assert!(monique.is_set());
    }

    #[test]
    fn display_label_falls_back() {
        let s = ManagedSiteInfo {
            tenant_id: "t".into(),
            site_id: "the-id".into(),
            name: String::new(),
            tenant_name: String::new(),
            host: String::new(),
            label: String::new(),
            favicon: None,
            brand_color: None,
            is_wordpress: None,
            wp_admin_url: None,
        };
        assert_eq!(s.display_label(), "the-id");
    }
}
