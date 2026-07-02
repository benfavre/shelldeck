//! Cloud Account — sign in to Inklura Manage (`manage.inklura.fr`) and bind a
//! cloud-sync token to the signed-in account.
//!
//! Three ways in, all landing on a `sd_…` bearer token:
//!   1. Password: [`login_password`] → `POST …/auth {action:"login",…}`.
//!   2. Browser / OIDC (device-authorize): the app binds a loopback listener,
//!      opens [`browser_connect_url`] in the system browser, and waits on
//!      [`browser_connect_listen`] for the redirect carrying the token.
//!   3. (identity refresh) [`whoami`] confirms a token is still valid and
//!      returns the account label / user.
//!
//! [`logout`] revokes the token server-side (best-effort).
//!
//! The listener parser is deliberately std-only — it reads a single HTTP
//! request line, so there is no need for a web-server dependency.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// The signed-in account identity, persisted in `AppConfig` `[account]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
}

impl AccountInfo {
    /// A one- or two-letter avatar initial derived from name/email.
    pub fn initial(&self) -> String {
        let src = if !self.name.trim().is_empty() {
            self.name.trim()
        } else {
            self.email.trim()
        };
        src.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }

    /// Best display name, falling back to the email local-part.
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.clone()
        } else if let Some((local, _)) = self.email.split_once('@') {
            local.to_string()
        } else if !self.email.is_empty() {
            self.email.clone()
        } else {
            "Compte".to_string()
        }
    }
}

/// User sub-object shared by the login and whoami responses. Fields can be
/// null/absent on older tokens.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountUser {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Parsed `?action=whoami` response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WhoamiInfo {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub user: Option<AccountUser>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub last_seen_at: Option<String>,
}

impl WhoamiInfo {
    /// Best-effort account identity from a whoami payload.
    pub fn account_info(&self) -> AccountInfo {
        let email = self
            .user
            .as_ref()
            .and_then(|u| u.email.clone())
            .unwrap_or_default();
        let name = self
            .user
            .as_ref()
            .and_then(|u| u.name.clone())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.label.clone().filter(|s| !s.trim().is_empty()))
            .unwrap_or_default();
        AccountInfo { email, name }
    }
}

#[derive(Debug, Default, Deserialize)]
struct LoginResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    user: Option<AccountUser>,
    #[serde(default)]
    error: Option<String>,
}

/// A user-facing message for an account error — unwraps the inner string of
/// our `Connection`/`Serialization` errors instead of showing the Display
/// prefix (e.g. "Connection error: …") in a toast.
pub fn user_message(err: &ShellDeckError) -> String {
    match err {
        ShellDeckError::Connection(m) | ShellDeckError::Serialization(m) => m.clone(),
        other => other.to_string(),
    }
}

/// True if `err` is an auth rejection (invalid/revoked token or bad creds),
/// as opposed to a transient network failure. Lets callers show a "reconnect"
/// hint (red status) rather than an "offline" one.
pub fn is_auth_rejected(err: &ShellDeckError) -> bool {
    matches!(err, ShellDeckError::Connection(m) if m.contains("(401)") || m.contains("(403)"))
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ShellDeckError::Connection(format!("failed to build HTTP client: {}", e)))
}

fn auth_url(base_url: &str) -> String {
    format!("{}/api/manage/shelldeck/auth", base_url.trim_end_matches('/'))
}

/// Best-effort device name for the auth check-in / connect flow.
pub fn device_name() -> String {
    if let Ok(h) = std::env::var("HOSTNAME") {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    "ShellDeck".to_string()
}

/// Password sign-in. Returns the account-bound bearer token and identity.
///
/// On any error the server's `error` string is surfaced verbatim (e.g. the
/// French "Email ou mot de passe incorrect." on 401).
pub fn login_password(
    base_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> Result<(String, AccountInfo)> {
    let client = http_client()?;
    let body = serde_json::json!({
        "action": "login",
        "email": email,
        "password": password,
        "device_name": device_name,
    });
    let resp = client
        .post(auth_url(base_url))
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("login request failed: {}", e)))?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    let parsed: Option<LoginResponse> = serde_json::from_str(&text).ok();

    if status.is_success() {
        if let Some(p) = &parsed {
            if p.ok {
                if let Some(token) = p.token.clone().filter(|t| !t.is_empty()) {
                    let account = p
                        .user
                        .as_ref()
                        .map(|u| AccountInfo {
                            email: u.email.clone().unwrap_or_else(|| email.to_string()),
                            name: u.name.clone().unwrap_or_default(),
                        })
                        .unwrap_or_else(|| AccountInfo {
                            email: email.to_string(),
                            name: String::new(),
                        });
                    return Ok((token, account));
                }
            }
        }
    }

    // Surface the server error string, tagging the HTTP status so callers can
    // distinguish auth rejection (401/403) from other failures.
    let server_msg = parsed
        .and_then(|p| p.error)
        .unwrap_or_else(|| format!("Échec de la connexion (HTTP {})", status.as_u16()));
    Err(ShellDeckError::Connection(format!(
        "{} ({})",
        server_msg,
        status.as_u16()
    )))
}

/// Confirm a token is valid and fetch the account identity.
pub fn whoami(base_url: &str, token: &str) -> Result<WhoamiInfo> {
    let client = http_client()?;
    let url = format!("{}?action=whoami", auth_url(base_url));
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("whoami request failed: {}", e)))?;

    let status = resp.status();
    if status.as_u16() == 401 {
        return Err(ShellDeckError::Connection(
            "session token rejected (401)".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(ShellDeckError::Connection(format!(
            "whoami failed: HTTP {}",
            status.as_u16()
        )));
    }
    resp.json::<WhoamiInfo>()
        .map_err(|e| ShellDeckError::Serialization(format!("invalid whoami payload: {}", e)))
}

/// Revoke a token server-side. Best-effort — callers should still clear local
/// state even if this errors.
pub fn logout(base_url: &str, token: &str) -> Result<()> {
    let client = http_client()?;
    let body = serde_json::json!({ "action": "logout" });
    let resp = client
        .post(auth_url(base_url))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| ShellDeckError::Connection(format!("logout request failed: {}", e)))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(ShellDeckError::Connection(format!(
            "logout failed: HTTP {}",
            resp.status().as_u16()
        )))
    }
}

/// Build the browser sign-in URL for the device-authorize flow.
///
/// `provider` is `sso`/`google`/`github`/`linkedin`, or `None` for the generic
/// flow (password login page).
pub fn browser_connect_url(
    base_url: &str,
    port: u16,
    state: &str,
    device: &str,
    provider: Option<&str>,
) -> String {
    let base = base_url.trim_end_matches('/');
    let mut url = format!(
        "{}/manage/shelldeck/connect?port={}&state={}&device={}",
        base,
        port,
        percent_encode(state),
        percent_encode(device),
    );
    if let Some(p) = provider {
        if !p.is_empty() {
            url.push_str("&provider=");
            url.push_str(&percent_encode(p));
        }
    }
    url
}

/// Wait for the browser to redirect to `http://127.0.0.1:<port>/callback?token=…&state=…`
/// on `listener`, verify the `state` matches, and return the token.
///
/// Bind the listener *before* calling (so the caller knows the port to embed in
/// the browser URL). Ignores stray requests (e.g. `/favicon.ico`) and
/// state-mismatched callbacks, continuing to listen until `timeout` elapses.
pub fn browser_connect_listen(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    listener
        .set_nonblocking(true)
        .map_err(ShellDeckError::Io)?;
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() >= deadline {
            return Err(ShellDeckError::Connection(
                "browser authorization timed out".to_string(),
            ));
        }
        match listener.accept() {
            Ok((stream, _addr)) => match handle_callback(stream, expected_state) {
                Ok(Some(token)) => return Ok(token),
                // favicon / state mismatch / malformed → keep waiting.
                Ok(None) | Err(_) => continue,
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(ShellDeckError::Io(e)),
        }
    }
}

const SUCCESS_HTML: &str = "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\">\
<title>ShellDeck</title><style>body{font-family:system-ui,sans-serif;background:#0f1115;\
color:#e6e6e6;display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}\
.c{text-align:center}.d{font-size:15px;color:#9aa4b2;margin-top:8px}h1{font-size:20px;margin:0}\
</style></head><body><div class=\"c\"><h1>ShellDeck connecté ✓</h1>\
<div class=\"d\">Vous pouvez fermer cet onglet.</div></div></body></html>";

const MISMATCH_HTML: &str = "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\">\
<title>ShellDeck</title></head><body style=\"font-family:system-ui,sans-serif\">\
<p>Requête de connexion invalide. Relancez la connexion depuis ShellDeck.</p></body></html>";

/// Parse one callback request on `stream`. Returns `Ok(Some(token))` only for a
/// `/callback` whose `state` matches; `Ok(None)` for anything else (so the
/// caller keeps listening).
fn handle_callback(mut stream: TcpStream, expected_state: &str) -> std::io::Result<Option<String>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let request_line = {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        line
    };

    // "GET /callback?token=…&state=… HTTP/1.1"
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    if path == "/callback" {
        let mut token: Option<String> = None;
        let mut state: Option<String> = None;
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "token" => token = Some(percent_decode(v)),
                    "state" => state = Some(percent_decode(v)),
                    _ => {}
                }
            }
        }
        if state.as_deref() == Some(expected_state) {
            if let Some(t) = token.filter(|t| !t.is_empty()) {
                write_response(&mut stream, 200, "OK", SUCCESS_HTML)?;
                return Ok(Some(t));
            }
        }
        write_response(&mut stream, 400, "Bad Request", MISMATCH_HTML)?;
        return Ok(None);
    }

    // /favicon.ico and everything else.
    write_response(&mut stream, 404, "Not Found", "Not found")?;
    Ok(None)
}

fn write_response(stream: &mut TcpStream, status: u16, reason: &str, body: &str) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}

/// Open `url` in the system browser (fire-and-forget; never blocks).
pub fn open_in_browser(url: &str) -> Result<()> {
    use std::process::Command;
    let spawn = |mut cmd: Command| {
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| ShellDeckError::Connection(format!("failed to open browser: {}", e)))
    };

    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        spawn(cmd)
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        spawn(cmd)
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        spawn(cmd)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = url;
        Err(ShellDeckError::Connection(
            "opening a browser is unsupported on this platform".to_string(),
        ))
    }
}

/// Percent-encode a query value (RFC 3986 unreserved chars pass through).
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Percent-decode `%XX` sequences (leaves anything else as-is).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn account_info_initial_and_display() {
        let a = AccountInfo {
            email: "ben@webdesign29.net".into(),
            name: "Ben Favre".into(),
        };
        assert_eq!(a.initial(), "B");
        assert_eq!(a.display_name(), "Ben Favre");

        let only_email = AccountInfo {
            email: "alice@example.com".into(),
            name: String::new(),
        };
        assert_eq!(only_email.initial(), "A");
        assert_eq!(only_email.display_name(), "alice");
    }

    #[test]
    fn whoami_account_info_falls_back_to_label() {
        let w = WhoamiInfo {
            ok: true,
            label: Some("Poste de Ben".into()),
            user: None,
            ..Default::default()
        };
        let info = w.account_info();
        assert_eq!(info.name, "Poste de Ben");
        assert!(info.email.is_empty());
    }

    #[test]
    fn browser_connect_url_encodes_and_appends_provider() {
        let url = browser_connect_url(
            "https://manage.inklura.fr/",
            41234,
            "abc_STATE-123",
            "Mac de Ben",
            Some("google"),
        );
        assert!(url.starts_with("https://manage.inklura.fr/manage/shelldeck/connect?"));
        assert!(url.contains("port=41234"));
        assert!(url.contains("state=abc_STATE-123"));
        assert!(url.contains("device=Mac%20de%20Ben"));
        assert!(url.ends_with("&provider=google"));

        let no_prov = browser_connect_url("https://x.test", 5000, "s", "d", None);
        assert!(!no_prov.contains("provider="));
    }

    #[test]
    fn percent_roundtrip() {
        assert_eq!(percent_decode("Mac%20de%20Ben"), "Mac de Ben");
        assert_eq!(percent_decode("sd_abcDEF-123_"), "sd_abcDEF-123_");
        assert_eq!(percent_decode("%2F%3D"), "/=");
        // Malformed trailing % is left as-is.
        assert_eq!(percent_decode("abc%"), "abc%");
    }

    #[test]
    fn is_auth_rejected_detects_401_403() {
        assert!(is_auth_rejected(&ShellDeckError::Connection(
            "session token rejected (401)".into()
        )));
        assert!(is_auth_rejected(&ShellDeckError::Connection(
            "Compte administrateur requis. (403)".into()
        )));
        assert!(!is_auth_rejected(&ShellDeckError::Connection(
            "login request failed: dns error".into()
        )));
    }

    fn send_callback(port: u16, request_target: &str) {
        // Best-effort client that fires one GET and reads the response.
        if let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = write!(
                s,
                "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
                request_target
            );
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
        }
    }

    #[test]
    fn browser_connect_returns_token_on_matching_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            browser_connect_listen(listener, "goodstate123", Duration::from_secs(5))
        });
        send_callback(port, "/callback?token=sd_abc123&state=goodstate123");
        let token = handle.join().unwrap().expect("token");
        assert_eq!(token, "sd_abc123");
    }

    #[test]
    fn browser_connect_ignores_wrong_state_and_favicon_then_accepts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            browser_connect_listen(listener, "goodstate", Duration::from_secs(5))
        });
        // Wrong state: must be ignored (processed + closed before the next).
        send_callback(port, "/callback?token=sd_wrong&state=badstate");
        // Stray favicon request: 404, keep listening.
        send_callback(port, "/favicon.ico");
        // Correct callback wins.
        send_callback(port, "/callback?token=sd_right&state=goodstate");
        let token = handle.join().unwrap().expect("token");
        assert_eq!(token, "sd_right");
    }

    #[test]
    fn browser_connect_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let start = Instant::now();
        let result = browser_connect_listen(listener, "state", Duration::from_millis(250));
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn browser_connect_percent_decodes_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            browser_connect_listen(listener, "s", Duration::from_secs(5))
        });
        send_callback(port, "/callback?token=sd%5Fabc&state=s");
        assert_eq!(handle.join().unwrap().unwrap(), "sd_abc");
    }
}
