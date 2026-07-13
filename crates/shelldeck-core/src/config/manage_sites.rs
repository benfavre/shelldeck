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
    /// JeanClaude dashboard config — delivered ONLY for super-admin tokens
    /// (`null`/absent otherwise). See `config::jeanclaude`.
    #[serde(default)]
    pub jeanclaude: Option<crate::config::jeanclaude::JeanConfig>,
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
          ]
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
