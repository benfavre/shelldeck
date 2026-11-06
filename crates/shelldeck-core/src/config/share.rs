//! Personal Inklura Share client. Manage's device credential is exchanged for a
//! one-use grant; only a five-minute Share session reaches Share. No API key is
//! requested or persisted. Large files use bounded, signed multipart uploads.
use reqwest::blocking::{Client, Response};
use reqwest::{redirect::Policy, Method};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const SHARE_ORIGIN: &str = "https://share.inklura.fr";
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const SINGLE_BYTES: u64 = 50 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Remote(String),
    #[error("network_unavailable")]
    Network(#[from] reqwest::Error),
    #[error("file_unavailable")]
    Io(#[from] std::io::Error),
}
pub type ShareResult<T> = std::result::Result<T, ShareError>;
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShareOwner {
    pub email: String,
    pub name: String,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SharePlan {
    pub id: String,
    pub label: String,
    pub max_file_bytes: u64,
    pub storage_bytes: u64,
    pub max_files: u64,
    pub retention_days: Vec<u64>,
    pub residency: String,
    pub storage_ready: bool,
    #[serde(default)]
    pub pending_france: bool,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShareUsage {
    pub bytes: u64,
    pub files: u64,
    #[serde(default)]
    pub pending_bytes: u64,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShareFile {
    pub id: String,
    pub filename: String,
    pub bytes: u64,
    pub content_type: String,
    pub kind: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub views: u64,
    pub url: String,
    pub viewer_url: String,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShareSnapshot {
    pub owner: ShareOwner,
    pub plan: SharePlan,
    pub usage: ShareUsage,
    pub default_retention_seconds: u64,
    pub files: Vec<ShareFile>,
    pub list_limit: usize,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SharedUpload {
    pub id: String,
    pub url: String,
    pub viewer: String,
}
#[derive(Clone)]
pub enum ShareSource {
    File(PathBuf),
    Bytes(Arc<Vec<u8>>),
}
#[derive(Clone)]
pub struct ShareUpload {
    pub filename: String,
    pub content_type: String,
    pub source: ShareSource,
}
impl ShareUpload {
    pub fn from_path(path: PathBuf) -> ShareResult<Self> {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(ShareError::Invalid("invalid_filename"))?
            .to_owned();
        let content_type = mime_for(&path)
            .ok_or(ShareError::Invalid("unsupported_type"))?
            .into();
        Ok(Self {
            filename,
            content_type,
            source: ShareSource::File(path),
        })
    }
    fn reader(&self) -> ShareResult<(Box<dyn Read>, u64)> {
        let (reader, length): (Box<dyn Read>, u64) = match &self.source {
            ShareSource::File(path) => {
                let file = std::fs::File::open(path)?;
                let metadata = file.metadata()?;
                if !metadata.is_file() {
                    return Err(ShareError::Invalid("invalid_file"));
                }
                let length = metadata.len();
                (Box::new(file), length)
            }
            ShareSource::Bytes(bytes) => (
                Box::new(std::io::Cursor::new(bytes.as_ref().clone())),
                bytes.len() as u64,
            ),
        };
        if length == 0 || length > MAX_FILE_BYTES {
            return Err(ShareError::Invalid("too_large"));
        }
        Ok((reader, length))
    }
}
fn mime_for(path: &Path) -> Option<&'static str> {
    Some(
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "avif" => "image/avif",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            "pdf" => "application/pdf",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "mov" => "video/quicktime",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            "wav" => "audio/wav",
            "txt" => "text/plain",
            "md" => "text/markdown",
            "csv" => "text/csv",
            "json" => "application/json",
            "log" => "text/x-log",
            "zip" => "application/zip",
            "gz" => "application/gzip",
            "7z" => "application/x-7z-compressed",
            _ => return None,
        },
    )
}
struct ShareSession {
    cookie: String,
    expires_at: u64,
}
// No Debug implementation: credentials must not appear in logs/errors.
#[derive(Clone)]
pub struct ShareClient {
    manage: String,
    origin: String,
    device: String,
    client: Client,
    cancelled: Arc<AtomicBool>,
}
impl ShareClient {
    pub fn new(manage: &str, device: &str, cancelled: Arc<AtomicBool>) -> ShareResult<Self> {
        let url =
            url::Url::parse(manage).map_err(|_| ShareError::Invalid("invalid_manage_origin"))?;
        if url.scheme() != "https"
            || url.username() != ""
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ShareError::Invalid("invalid_manage_origin"));
        }
        Ok(Self {
            manage: manage.trim_end_matches('/').into(),
            origin: SHARE_ORIGIN.into(),
            device: device.into(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(180))
                .redirect(Policy::none())
                .build()?,
            cancelled,
        })
    }
    fn check(&self) -> ShareResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(ShareError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn decode<T: serde::de::DeserializeOwned>(response: Response) -> ShareResult<T> {
        let status = response.status();
        // Never include a server HTML body, credential or signed URL in errors.
        if status.as_u16() == 401 {
            return Err(ShareError::Remote("share_session_rejected".into()));
        }
        let mut bytes = Vec::new();
        response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(ShareError::Invalid("invalid_response"));
        }
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|_| ShareError::Invalid("invalid_response"))?;
        if !status.is_success() {
            return Err(ShareError::Remote(
                body.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("share_unavailable")
                    .chars()
                    .take(100)
                    .collect(),
            ));
        }
        serde_json::from_value(body).map_err(|_| ShareError::Invalid("invalid_response"))
    }
    fn authenticate(&self) -> ShareResult<ShareSession> {
        self.check()?;
        let response = self
            .client
            .post(format!("{}/api/manage/shelldeck/share", self.manage))
            .timeout(Duration::from_secs(15))
            .bearer_auth(&self.device)
            .json(&json!({"action":"connect"}))
            .send()?;
        self.check()?;
        // Only Manage can establish that the saved device credential is revoked.
        // An expired one-use grant or Share outage must not log out other tools.
        if response.status().as_u16() == 401 {
            return Err(ShareError::Unauthorized);
        }
        let grant: Value = Self::decode(response)?;
        let token = grant["grant"]
            .as_str()
            .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(ShareError::Invalid("invalid_grant"))?;
        if grant["share_origin"].as_str() != Some(self.origin.as_str()) {
            return Err(ShareError::Invalid("invalid_share_origin"));
        }
        self.check()?;
        let session: Value = Self::decode(
            self.client
                .post(format!("{}/api/desktop/session", self.origin))
                .timeout(Duration::from_secs(15))
                .json(&json!({"grant":token}))
                .send()?,
        )?;
        self.check()?;
        let cookie = session["session"]
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.len() < 8192
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
            .ok_or(ShareError::Invalid("invalid_session"))?;
        let expires_at = session["expires_at"]
            .as_u64()
            .filter(|expires| *expires > now_ms() && *expires <= now_ms() + 360000)
            .ok_or(ShareError::Invalid("invalid_session"))?;
        Ok(ShareSession {
            cookie: format!("share_session={cookie}"),
            expires_at,
        })
    }
    fn json<T: serde::de::DeserializeOwned>(
        &self,
        session: &ShareSession,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> ShareResult<T> {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.origin, path))
            .timeout(Duration::from_secs(15))
            .header("cookie", &session.cookie)
            .header("origin", &self.origin);
        if let Some(body) = body {
            request = request.json(&body);
        }
        Self::decode(request.send()?)
    }
    pub fn snapshot(&self) -> ShareResult<ShareSnapshot> {
        let session = self.authenticate()?;
        let snapshot: ShareSnapshot = self.json(&session, Method::GET, "/api/desktop", None)?;
        if snapshot.files.len() > 500
            || snapshot.plan.retention_days.len() > 20
            || snapshot.files.iter().any(|file| {
                !valid_id(&file.id) || !share_url(&file.viewer_url) || !share_url(&file.url)
            })
        {
            return Err(ShareError::Invalid("invalid_response"));
        }
        self.check()?;
        Ok(snapshot)
    }
    pub fn delete(&self, id: &str) -> ShareResult<()> {
        if !valid_id(id) {
            return Err(ShareError::Invalid("invalid_file"));
        }
        let session = self.authenticate()?;
        self.check()?;
        let result: Value = self.json(
            &session,
            Method::POST,
            "/api/uploads/action",
            Some(json!({"op":"delete","id":id})),
        )?;
        if result["ok"] != true {
            return Err(ShareError::Invalid("delete_failed"));
        }
        Ok(())
    }
    pub fn set_retention(&self, seconds: u64) -> ShareResult<()> {
        if !seconds.is_multiple_of(86400) || seconds > 365 * 86400 {
            return Err(ShareError::Invalid("invalid_retention"));
        }
        let session = self.authenticate()?;
        self.check()?;
        let response = self
            .client
            .post(format!("{}/api/settings", self.origin))
            .header("cookie", &session.cookie)
            .header("origin", &self.origin)
            .form(&[
                ("action", "retention"),
                ("retentionDays", &(seconds / 86400).to_string()),
            ])
            .send()?;
        if response.status().as_u16() == 303
            && response
                .headers()
                .get("location")
                .and_then(|h| h.to_str().ok())
                == Some("/settings?saved=1")
        {
            Ok(())
        } else {
            Err(ShareError::Invalid("invalid_retention"))
        }
    }
    pub fn upload(
        &self,
        upload: &ShareUpload,
        retention: u64,
        mut progress: impl FnMut(u64, u64),
    ) -> ShareResult<SharedUpload> {
        let (mut reader, length) = upload.reader()?;
        if retention > 365 * 86400 || (retention != 0 && retention < 300) {
            return Err(ShareError::Invalid("invalid_retention"));
        }
        let mut session = self.authenticate()?;
        self.check()?;
        let result: SharedUpload = if length <= SINGLE_BYTES {
            let mut bytes = Vec::new();
            reader.take(length + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 != length {
                return Err(ShareError::Invalid("file_changed"));
            }
            let form = reqwest::blocking::multipart::Form::new()
                .part(
                    "file",
                    reqwest::blocking::multipart::Part::bytes(bytes)
                        .file_name(upload.filename.clone())
                        .mime_str(&upload.content_type)?,
                )
                .text("expires_in", retention.to_string());
            self.check()?;
            Self::decode(
                self.client
                    .post(format!("{}/api/upload", self.origin))
                    .header("cookie", &session.cookie)
                    .header("origin", &self.origin)
                    .multipart(form)
                    .send()?,
            )?
        } else {
            let started: Value = self.json(&session, Method::POST, "/api/upload/resumable", Some(json!({"filename":upload.filename,"content_type":upload.content_type,"bytes":length,"expires_in":retention})))?;
            let id = started["id"]
                .as_str()
                .filter(|id| id.len() == 48 && id.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or(ShareError::Invalid("invalid_upload_session"))?
                .to_owned();
            let outcome = (|| {
                let part_bytes = started["part_bytes"]
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
                    .ok_or(ShareError::Invalid("invalid_upload_session"))?;
                let parts = length.div_ceil(part_bytes);
                if started["parts"].as_u64() != Some(parts) {
                    return Err(ShareError::Invalid("invalid_upload_session"));
                }
                let mut done = 0;
                for number in 1..=parts {
                    self.check()?;
                    if session.expires_at < now_ms() + 30000 {
                        session = self.authenticate()?;
                    }
                    let part: Value = self.json(
                        &session,
                        Method::POST,
                        &format!("/api/upload/resumable/{id}/part"),
                        Some(json!({"part":number})),
                    )?;
                    let url = part["url"]
                        .as_str()
                        .filter(|url| storage_url(url))
                        .ok_or(ShareError::Invalid("invalid_storage_origin"))?;
                    let size = (length - done).min(part_bytes);
                    let mut bytes = vec![0; size as usize];
                    reader.read_exact(&mut bytes)?;
                    self.check()?;
                    let response = self
                        .client
                        .put(url)
                        .header("origin", &self.origin)
                        .body(bytes)
                        .send()?;
                    if !response.status().is_success() {
                        return Err(ShareError::Invalid("part_upload_failed"));
                    }
                    done += size;
                    progress(done, length);
                }
                let mut extra = [0];
                if reader.read(&mut extra)? != 0 {
                    return Err(ShareError::Invalid("file_changed"));
                }
                self.check()?;
                if session.expires_at < now_ms() + 30000 {
                    session = self.authenticate()?;
                }
                self.check()?;
                self.json(
                    &session,
                    Method::POST,
                    &format!("/api/upload/resumable/{id}/complete"),
                    Some(json!({})),
                )
            })();
            if outcome.is_err() {
                let _: ShareResult<Value> = self.json(
                    &session,
                    Method::POST,
                    &format!("/api/upload/resumable/{id}/abort"),
                    Some(json!({})),
                );
            }
            outcome?
        };
        let result: SharedUpload = result;
        if !valid_id(&result.id) || !share_url(&result.url) || !share_url(&result.viewer) {
            return Err(ShareError::Invalid("invalid_response"));
        }
        progress(length, length);
        Ok(result)
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn valid_id(id: &str) -> bool {
    (6..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn share_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.origin().ascii_serialization() == SHARE_ORIGIN
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}
fn storage_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        let host = url.host_str().unwrap_or_default();
        url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && (host.ends_with(".r2.cloudflarestorage.com")
                || host == "s3.eu-west-3.amazonaws.com"
                || host.ends_with(".s3.eu-west-3.amazonaws.com"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};
    fn fixture(responses: Vec<Value>) -> (ShareClient, thread::JoinHandle<Vec<String>>) {
        fixture_with_cancel(responses, Arc::new(AtomicBool::new(false)))
    }
    fn fixture_with_cancel(
        responses: Vec<Value>,
        cancelled: Arc<AtomicBool>,
    ) -> (ShareClient, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let fixture_origin = origin.clone();
        let server_cancelled = cancelled.clone();
        let worker = thread::spawn(move || {
            let mut requests = vec![];
            for mut response in responses {
                if response["share_origin"] == "$origin" {
                    response["share_origin"] = fixture_origin.clone().into();
                }
                if response["expires_at"] == "$expiry" {
                    response["expires_at"] = (now_ms() + 300000).into();
                }
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut block = [0; 4096];
                loop {
                    let n = stream.read(&mut block).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&block[..n]);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&bytes).into_owned());
                if response["$cancel"] == true {
                    server_cancelled.store(true, Ordering::Release);
                }
                let status = response["$status"].as_u64().unwrap_or(200);
                let body = response.to_string();
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            requests
        });
        (
            ShareClient {
                manage: origin.clone(),
                origin,
                device: "sd_synthetic".into(),
                client: Client::builder().redirect(Policy::none()).build().unwrap(),
                cancelled,
            },
            worker,
        )
    }
    fn grant() -> Value {
        json!({"grant":"a".repeat(64),"share_origin":"$origin"})
    }
    fn session() -> Value {
        json!({"session":"synthetic.payload","expires_at":"$expiry"})
    }
    // SDTEST-2889 / SDUC-568
    #[test]
    fn sdtest_2889_personal_share_auth_uses_separate_manage_and_share_credentials() {
        let payload = json!({"owner":{"email":"owner@example.test","name":"Owner"},"plan":{"id":"free","label":"Gratuit","max_file_bytes":104857600,"storage_bytes":1073741824,"max_files":1000,"retention_days":[1,7],"residency":"global","storage_ready":true},"usage":{"bytes":12,"files":1},"default_retention_seconds":604800,"list_limit":500,"files":[{"id":"abcdef","filename":"mine.pdf","bytes":12,"content_type":"application/pdf","kind":"pdf","created_at":"2026-11-06T12:00:00Z","expires_at":null,"views":0,"url":"https://share.inklura.fr/r/abcdef.pdf","viewer_url":"https://share.inklura.fr/s/abcdef"}]});
        let (client, worker) = fixture(vec![grant(), session(), payload]);
        let snapshot = client.snapshot().unwrap();
        assert_eq!(snapshot.files[0].filename, "mine.pdf");
        assert_eq!(snapshot.default_retention_seconds, 604800);
        let requests = worker.join().unwrap();
        assert!(requests[0].starts_with("POST /api/manage/shelldeck/share "));
        assert!(requests[0].contains("Bearer sd_synthetic"));
        assert!(requests[1].starts_with("POST /api/desktop/session "));
        assert!(!requests[1].contains("sd_synthetic"));
        assert!(requests[2].starts_with("GET /api/desktop "));
        assert!(requests[2].contains("share_session=synthetic.payload"));
        assert!(!requests[2].contains("sd_synthetic"));
    }
    // SDTEST-2890 / SDUC-568
    #[test]
    fn sdtest_2890_untrusted_origin_and_cancelled_session_stop_before_upload() {
        let (client, worker) = fixture(vec![
            json!({"grant":"a".repeat(64),"share_origin":"https://attacker.example"}),
        ]);
        assert!(matches!(
            client.snapshot(),
            Err(ShareError::Invalid("invalid_share_origin"))
        ));
        assert_eq!(worker.join().unwrap().len(), 1);
        client.cancelled.store(true, Ordering::Release);
        assert!(matches!(client.snapshot(), Err(ShareError::Cancelled)));
        assert!(!share_url(
            "https://share.inklura.fr.attacker.example/s/abcdef"
        ));
        assert!(!share_url(
            "https://share.inklura.fr/s/abcdef?session=secret"
        ));
        assert!(!storage_url("http://bucket.r2.cloudflarestorage.com/a"));
        assert!(!storage_url(
            "https://x.r2.cloudflarestorage.com.attacker.example/a"
        ));
    }
    // SDTEST-2891 / SDUC-568
    #[test]
    fn sdtest_2891_large_file_aborts_reservation_when_part_origin_is_invalid() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.zip");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(51 * 1024 * 1024)
            .unwrap();
        let id = "b".repeat(48);
        let (client, worker) = fixture(vec![
            grant(),
            session(),
            json!({"id":id,"part_bytes":8388608,"parts":7}),
            json!({"url":"https://attacker.example/receive"}),
            json!({"ok":true}),
        ]);
        let upload = ShareUpload::from_path(path).unwrap();
        assert!(matches!(
            client.upload(&upload, 604800, |_, _| {}),
            Err(ShareError::Invalid("invalid_storage_origin"))
        ));
        let requests = worker.join().unwrap();
        assert!(requests[2].contains("\"bytes\":53477376"));
        assert!(requests[4].starts_with(&format!("POST /api/upload/resumable/{id}/abort ")));
        let (client, worker) = fixture(vec![
            grant(),
            session(),
            json!({"id":id,"parts":7}),
            json!({"ok":true}),
        ]);
        assert!(matches!(
            client.upload(&upload, 604800, |_, _| {}),
            Err(ShareError::Invalid("invalid_upload_session"))
        ));
        assert!(worker.join().unwrap()[3]
            .starts_with(&format!("POST /api/upload/resumable/{id}/abort ")));
    }
    // SDTEST-2894 / SDUC-568
    #[test]
    fn sdtest_2894_only_revoked_manage_devices_invalidate_login_and_mint_cancellation_stops_work() {
        for (responses, revoked) in [
            (vec![json!({"$status":401})], true),
            (
                vec![json!({"$status":503,"error":"share_unavailable"})],
                false,
            ),
            (vec![grant(), json!({"$status":401})], false),
            (
                vec![grant(), json!({"$status":503,"error":"share_unavailable"})],
                false,
            ),
        ] {
            let (client, worker) = fixture(responses);
            let error = client.snapshot().unwrap_err();
            assert_eq!(matches!(error, ShareError::Unauthorized), revoked);
            worker.join().unwrap();
        }
        let mut minted = session();
        minted["$cancel"] = true.into();
        let (client, worker) = fixture(vec![grant(), minted]);
        assert!(matches!(client.snapshot(), Err(ShareError::Cancelled)));
        assert_eq!(worker.join().unwrap().len(), 2);
    }
}
