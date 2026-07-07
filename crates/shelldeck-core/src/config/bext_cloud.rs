//! cloud.bext.dev integration — the hosted bext control plane.
//!
//! ShellDeck connects to a cloud.bext.dev account with a CLI token (`bext_…`,
//! Bearer auth) minted by the site's OIDC CLI flow, then lists/creates/manages
//! hosted WordPress sites, reads the dashboard, and (as a super-admin) lists the
//! bext instances the cloud knows about.
//!
//! Connect flow (reuses the loopback listener from [`crate::config::cloud_account`]):
//!   1. Bind `127.0.0.1:0` → port P; open `https://cloud.bext.dev/api/auth/cli?port=P`.
//!   2. The user logs in via OIDC; the callback redirects the browser to
//!      `http://127.0.0.1:P/callback?token=bext_…&email=…&user_id=…&name=…`.
//!   3. We read the token + identity off that request. NOTE: unlike the manage
//!      device-connect flow, cloud's CLI flow carries **no `state`** (it uses a
//!      server-side port cookie), so we accept `/callback` without a state check.
//!
//! HTTP is `reqwest` blocking with the crate's usual 4s/10s(ish) timeouts; a 401
//! is surfaced as a clear "token rejected". All list/read types are tolerant of
//! missing/null fields (the dashboard payload is broad and partly optional).

use crate::config::cloud_account::percent_decode;
use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Persisted `[bext_cloud]` config section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BextCloudConfig {
    /// Base URL of the control plane.
    pub base_url: String,
    /// CLI token (`bext_…`). Empty = not connected.
    pub token: String,
    /// Signed-in identity (display only).
    pub email: String,
    pub name: String,
}

impl Default for BextCloudConfig {
    fn default() -> Self {
        Self {
            base_url: "https://cloud.bext.dev".to_string(),
            token: String::new(),
            email: String::new(),
            name: String::new(),
        }
    }
}

impl BextCloudConfig {
    pub fn is_connected(&self) -> bool {
        !self.token.is_empty() && !self.base_url.is_empty()
    }
}

// ── payload types (defensive) ─────────────────────────────────────────────────

fn de_nullable_string<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CloudUser {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub email: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default)]
    pub is_super_admin: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WhoamiResponse {
    #[serde(default)]
    user: CloudUser,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CloudSite {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub slug: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub kind: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub env: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub primary_domain: String,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub orphaned: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SitesResponse {
    #[serde(default)]
    pub sites: Vec<CloudSite>,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub max: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CloudStats {
    #[serde(default)]
    pub projects: u32,
    #[serde(default)]
    pub deploys: u32,
    #[serde(default)]
    pub domains: u32,
    #[serde(default)]
    pub targets: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DashboardResponse {
    #[serde(default)]
    pub user: CloudUser,
    #[serde(default)]
    pub stats: CloudStats,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CloudInstance {
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub host: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub url: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub status: String,
    #[serde(default, deserialize_with = "de_nullable_string")]
    pub health: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstancesResponse {
    #[serde(default)]
    pub instances: Vec<CloudInstance>,
    #[serde(default)]
    pub total: u32,
}

// ── HTTP ──────────────────────────────────────────────────────────────────────

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

fn map_status(status: u16) -> Option<ShellDeckError> {
    match status {
        200..=299 => None,
        401 => Some(ShellDeckError::Connection(
            "cloud token rejected (401) — reconnect to cloud.bext.dev".to_string(),
        )),
        403 => Some(ShellDeckError::Connection(
            "forbidden (403) — super-admin required".to_string(),
        )),
        s => Some(ShellDeckError::Connection(format!(
            "cloud request failed: HTTP {}",
            s
        ))),
    }
}

fn api(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

// A single retry on a *transport* error (never on an HTTP status) absorbs the
// occasional loopback/proxy connection reset seen on this host.
fn send_retry(
    build: impl Fn() -> reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::Response> {
    match build().send() {
        Ok(r) => Ok(r),
        Err(_) => {
            std::thread::sleep(Duration::from_millis(150));
            build()
                .send()
                .map_err(|e| ShellDeckError::Connection(format!("cloud request failed: {}", e)))
        }
    }
}

fn get_json<T: serde::de::DeserializeOwned>(cfg: &BextCloudConfig, path: &str) -> Result<T> {
    let client = http_client()?;
    let url = api(&cfg.base_url, path);
    let resp = send_retry(|| client.get(&url).bearer_auth(&cfg.token))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    resp.json::<T>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid cloud response: {}", e)))
}

fn post_json<T: serde::de::DeserializeOwned>(
    cfg: &BextCloudConfig,
    path: &str,
    body: &serde_json::Value,
) -> Result<T> {
    let client = http_client()?;
    let url = api(&cfg.base_url, path);
    let resp = send_retry(|| client.post(&url).bearer_auth(&cfg.token).json(body))?;
    if let Some(e) = map_status(resp.status().as_u16()) {
        return Err(e);
    }
    resp.json::<T>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid cloud response: {}", e)))
}

/// Verify the token + return the signed-in identity.
pub fn whoami(cfg: &BextCloudConfig) -> Result<CloudUser> {
    Ok(get_json::<WhoamiResponse>(cfg, "/api/auth/whoami")?.user)
}

/// List the account's managed sites.
pub fn list_sites(cfg: &BextCloudConfig) -> Result<SitesResponse> {
    get_json(cfg, "/api/sites")
}

/// Create a one-click WordPress site (`name` = lowercase slug).
pub fn create_site(
    cfg: &BextCloudConfig,
    name: &str,
    title: Option<&str>,
) -> Result<serde_json::Value> {
    let mut body = serde_json::json!({ "name": name.trim().to_lowercase() });
    if let Some(t) = title {
        body["title"] = serde_json::json!(t);
    }
    post_json(cfg, "/api/sites", &body)
}

/// Run a per-site lifecycle action: `go_live` | `config` | `destroy`.
pub fn site_action(
    cfg: &BextCloudConfig,
    slug: &str,
    action: &str,
    extra: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut body = serde_json::json!({ "slug": slug, "action": action });
    if let Some(serde_json::Value::Object(map)) = extra {
        for (k, v) in map {
            body[k] = v;
        }
    }
    post_json(cfg, "/api/sites/action", &body)
}

/// Dashboard overview (identity + stats).
pub fn dashboard(cfg: &BextCloudConfig) -> Result<DashboardResponse> {
    get_json(cfg, "/api/dashboard")
}

/// The bext instances the cloud knows about (super-admin only → 403 otherwise).
pub fn list_instances(cfg: &BextCloudConfig) -> Result<InstancesResponse> {
    get_json(cfg, "/api/admin/instances")
}

// ── browser CLI connect (loopback) ────────────────────────────────────────────

/// The URL to open in the system browser to start the cloud CLI login.
pub fn cli_login_url(base_url: &str, port: u16) -> String {
    format!(
        "{}/api/auth/cli?port={}",
        base_url.trim_end_matches('/'),
        port
    )
}

/// What the loopback callback hands back after a successful cloud CLI login.
#[derive(Debug, Clone)]
pub struct CloudConnect {
    pub token: String,
    pub email: String,
    pub name: String,
}

const OK_HTML: &str = "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\">\
<title>ShellDeck ↔ bext Cloud</title><style>body{font-family:system-ui,sans-serif;background:#0f1115;\
color:#e6e6e6;display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}\
.c{text-align:center}h1{font-size:20px;margin:0}.d{color:#9aa4b2;margin-top:8px}</style></head>\
<body><div class=\"c\"><h1>bext Cloud connecté ✓</h1><div class=\"d\">Vous pouvez fermer cet onglet.</div></div></body></html>";

/// Block on the loopback listener until the cloud CLI callback arrives (or timeout).
/// Bind the listener OUTSIDE (so the caller knows `port` before opening the browser).
pub fn browser_connect_listen(listener: TcpListener, timeout: Duration) -> Result<CloudConnect> {
    listener.set_nonblocking(true).map_err(ShellDeckError::Io)?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(ShellDeckError::Connection(
                "cloud authorization timed out".to_string(),
            ));
        }
        match listener.accept() {
            Ok((stream, _addr)) => match handle_callback(stream) {
                Ok(Some(c)) => return Ok(c),
                Ok(None) | Err(_) => continue, // favicon / malformed → keep waiting
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(ShellDeckError::Io(e)),
        }
    }
}

fn handle_callback(mut stream: TcpStream) -> std::io::Result<Option<CloudConnect>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request_line = {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        line
    };
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        write_response(&mut stream, 404, "Not Found", "Not found")?;
        return Ok(None);
    }
    let (mut token, mut email, mut name) = (None, None, None);
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "token" => token = Some(percent_decode(v)),
                "email" => email = Some(percent_decode(v)),
                "name" => name = Some(percent_decode(v)),
                _ => {}
            }
        }
    }
    if let Some(t) = token.filter(|t| t.starts_with("bext_")) {
        write_response(&mut stream, 200, "OK", OK_HTML)?;
        return Ok(Some(CloudConnect {
            token: t,
            email: email.unwrap_or_default(),
            name: name.unwrap_or_default(),
        }));
    }
    write_response(&mut stream, 400, "Bad Request", "invalid callback")?;
    Ok(None)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status, reason, body.len(), body
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn config_default_and_connected() {
        let mut c = BextCloudConfig::default();
        assert_eq!(c.base_url, "https://cloud.bext.dev");
        assert!(!c.is_connected());
        c.token = "bext_abc".into();
        assert!(c.is_connected());
    }

    #[test]
    fn cli_url_shape() {
        assert_eq!(
            cli_login_url("https://cloud.bext.dev/", 45123),
            "https://cloud.bext.dev/api/auth/cli?port=45123"
        );
    }

    #[test]
    fn parses_sites_with_nulls() {
        let j = r#"{"sites":[{"slug":"s1","kind":"wordpress","env":"staging","status":null,"primary_domain":"s1.staging.bext.dev","domains":["s1.staging.bext.dev"],"orphaned":false}],"count":1,"max":8}"#;
        let r: SitesResponse = serde_json::from_str(j).unwrap();
        assert_eq!(r.count, 1);
        assert_eq!(r.sites[0].slug, "s1");
        assert_eq!(r.sites[0].status, "");
    }

    #[test]
    fn parses_dashboard_and_instances() {
        let d: DashboardResponse = serde_json::from_str(r#"{"user":{"id":"u","email":"a@b","name":"A","is_super_admin":true},"stats":{"projects":2,"deploys":5,"domains":1,"targets":0}}"#).unwrap();
        assert!(d.user.is_super_admin);
        assert_eq!(d.stats.deploys, 5);
        let i: InstancesResponse = serde_json::from_str(r#"{"instances":[{"id":"x","name":"activ-2","host":"h","url":"https://h","status":"unreachable","health":"connected"}],"total":1}"#).unwrap();
        assert_eq!(i.total, 1);
        assert_eq!(i.instances[0].name, "activ-2");
    }

    fn send_callback(port: u16, target: &str) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ =
            s.write_all(format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", target).as_bytes());
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf);
    }

    #[test]
    fn browser_connect_returns_token() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || browser_connect_listen(l, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(80));
        send_callback(
            port,
            "/callback?token=bext_deadbeef&email=a%40b.com&name=Ben",
        );
        let c = h.join().unwrap().unwrap();
        assert_eq!(c.token, "bext_deadbeef");
        assert_eq!(c.email, "a@b.com");
        assert_eq!(c.name, "Ben");
    }

    #[test]
    fn browser_connect_ignores_favicon_then_accepts() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || browser_connect_listen(l, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(80));
        send_callback(port, "/favicon.ico");
        send_callback(port, "/callback?token=bext_zzz&email=x&name=y");
        assert_eq!(h.join().unwrap().unwrap().token, "bext_zzz");
    }
}
