//! Single bext instance management — the site-lifecycle SDK a bext server exposes
//! on its loopback (`http://127.0.0.1/__bext/sdk/site/*`), gated by the loopback
//! bypass + an `X-Bext-App-Id` header. ShellDeck manages the sites on a bext box
//! behind an SSH connection: open a tunnel to the box's loopback `:80`, point
//! [`BextInstance`] at the local tunnel endpoint, and drive the SDK.
//!
//! This is the "manage a single bext instance directly" surface (vs the hosted
//! cloud.bext.dev control plane in [`crate::config::bext_cloud`]).

use crate::error::{Result, ShellDeckError};
use serde::Deserialize;
use std::time::Duration;

fn de_nullable_string<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// A reachable bext instance SDK endpoint: the base URL (a local tunnel to the
/// box's loopback, e.g. `http://127.0.0.1:18080`, or a direct URL) + the app id
/// sent as `X-Bext-App-Id`.
#[derive(Debug, Clone)]
pub struct BextInstance {
    pub base_url: String,
    pub app_id: String,
}

impl BextInstance {
    pub fn new(base_url: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            app_id: app_id.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstanceSite {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub slug: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub kind: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub title: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub env: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub primary_domain: String,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub unix_user: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub db_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstanceSites {
    #[serde(default)]
    pub sites: Vec<InstanceSite>,
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

fn sdk_url(inst: &BextInstance, path: &str) -> String {
    format!(
        "{}/__bext/sdk/site{}",
        inst.base_url.trim_end_matches('/'),
        path
    )
}

fn map_status(status: u16) -> Option<ShellDeckError> {
    match status {
        200..=299 => None,
        401 | 403 => Some(ShellDeckError::Connection(
            "instance SDK rejected the request (auth) — is the tunnel to loopback?".to_string(),
        )),
        s => Some(ShellDeckError::Connection(format!(
            "instance SDK request failed: HTTP {}",
            s
        ))),
    }
}

// The bext loopback (:80) occasionally resets a fresh connection under load — a
// single retry on a *transport* error (never on an HTTP status) absorbs the blip.
fn send_retry(build: impl Fn() -> reqwest::blocking::RequestBuilder) -> Result<reqwest::blocking::Response> {
    match build().send() {
        Ok(r) => Ok(r),
        Err(_) => {
            std::thread::sleep(Duration::from_millis(150));
            build()
                .send()
                .map_err(|e| ShellDeckError::Connection(format!("instance SDK request failed: {}", e)))
        }
    }
}

fn get_json<T: serde::de::DeserializeOwned>(inst: &BextInstance, path: &str) -> Result<T> {
    let client = http_client()?;
    let url = sdk_url(inst, path);
    let resp = send_retry(|| client.get(&url).header("x-bext-app-id", &inst.app_id))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    resp.json::<T>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid instance SDK response: {}", e)))
}

fn post_json<T: serde::de::DeserializeOwned>(inst: &BextInstance, path: &str, body: &serde_json::Value) -> Result<T> {
    let client = http_client()?;
    let url = sdk_url(inst, path);
    let resp = send_retry(|| client.post(&url).header("x-bext-app-id", &inst.app_id).json(body))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    resp.json::<T>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid instance SDK response: {}", e)))
}

/// List the sites on the instance.
pub fn list_sites(inst: &BextInstance) -> Result<InstanceSites> {
    get_json(inst, "/list")
}

/// Get one site by slug.
pub fn get_site(inst: &BextInstance, slug: &str) -> Result<serde_json::Value> {
    get_json(
        inst,
        &format!("/get?slug={}", crate::config::cloud_account::percent_encode(slug)),
    )
}

/// Provision a new site (`kind` e.g. "wordpress", `env` e.g. "staging").
pub fn create_site(inst: &BextInstance, slug: &str, title: Option<&str>, kind: Option<&str>, env: Option<&str>) -> Result<serde_json::Value> {
    let mut body = serde_json::json!({ "slug": slug });
    if let Some(t) = title { body["title"] = serde_json::json!(t); }
    if let Some(k) = kind { body["kind"] = serde_json::json!(k); }
    if let Some(e) = env { body["env"] = serde_json::json!(e); }
    post_json(inst, "/create", &body)
}

/// Attach a live domain to a site.
pub fn go_live(inst: &BextInstance, slug: &str, domain: &str) -> Result<serde_json::Value> {
    post_json(inst, "/go_live", &serde_json::json!({ "slug": slug, "domain": domain }))
}

/// Update a site's config (arbitrary fields merged server-side).
pub fn config_site(inst: &BextInstance, slug: &str, extra: serde_json::Value) -> Result<serde_json::Value> {
    let mut body = serde_json::json!({ "slug": slug });
    if let serde_json::Value::Object(map) = extra {
        for (k, v) in map { body[k] = v; }
    }
    post_json(inst, "/config", &body)
}

/// Destroy a site.
pub fn destroy_site(inst: &BextInstance, slug: &str) -> Result<serde_json::Value> {
    post_json(inst, "/destroy", &serde_json::json!({ "slug": slug }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // Minimal one-shot HTTP mock serving a canned body.
    fn mock(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
            req
        });
        (addr, h)
    }

    #[test]
    fn list_sites_parses_and_sends_app_id() {
        let (addr, h) = mock(r#"{"sites":[{"slug":"testcool","kind":"wordpress","title":"testcool","env":"staging","primary_domain":"testcool.staging.bext.dev","domains":["testcool.staging.bext.dev"],"unix_user":"wp-testcool","db_name":null}]}"#);
        let inst = BextInstance::new(addr, "cloud-bext");
        let r = list_sites(&inst).unwrap();
        assert_eq!(r.sites.len(), 1);
        assert_eq!(r.sites[0].slug, "testcool");
        assert_eq!(r.sites[0].db_name, ""); // null tolerated
        let req = h.join().unwrap();
        assert!(req.contains("/__bext/sdk/site/list"));
        assert!(req.to_lowercase().contains("x-bext-app-id: cloud-bext"));
    }

    #[test]
    fn create_body_shape() {
        let (addr, h) = mock(r#"{"ok":true}"#);
        let inst = BextInstance::new(addr, "app");
        let _ = create_site(&inst, "newsite", Some("New Site"), Some("wordpress"), Some("staging")).unwrap();
        let req = h.join().unwrap();
        assert!(req.contains("/__bext/sdk/site/create"));
        assert!(req.contains("\"slug\":\"newsite\""));
        assert!(req.contains("\"kind\":\"wordpress\""));
    }
}
