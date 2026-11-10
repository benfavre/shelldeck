//! Native projection of Manage's scoped workflow and schedule APIs.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct AutomationError {
    pub status: u16,
    pub needs_reconnect: bool,
    pub message: String,
}
type Result<T> = std::result::Result<T, AutomationError>;
fn error(message: impl Into<String>) -> AutomationError {
    AutomationError {
        status: 0,
        needs_reconnect: false,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, rename = "nextRunAt")]
    pub next_run: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub workflow_id: String,
    #[serde(default)]
    pub node_id: String,
    pub cron: String,
    pub timezone: String,
    pub is_active: bool,
    #[serde(default)]
    pub next_run: Option<String>,
    #[serde(default)]
    pub last_run: Option<String>,
    #[serde(default)]
    pub run_count: u64,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Run {
    pub id: String,
    #[serde(rename = "workflowId")]
    pub workflow_id: String,
    pub name: String,
    pub status: String,
    #[serde(default, rename = "startedAt")]
    pub started_at: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AutomationData {
    #[serde(default)]
    pub tasks: Vec<Workflow>,
    #[serde(default)]
    pub task_total: usize,
    #[serde(default)]
    pub workflows: Vec<Workflow>,
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub schedule_total: usize,
    #[serde(default)]
    pub run_total: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskDraft {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub name: String,
    pub prompt: String,
    pub cron: String,
    pub timezone: String,
    pub once: bool,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskEditor {
    pub id: String,
    pub name: String,
    pub status: String,
    pub revision: String,
    pub editable: bool,
    pub prompt: String,
    pub cron: String,
    pub timezone: String,
    pub once: bool,
    #[serde(default)]
    pub has_unpublished_changes: bool,
    #[serde(default)]
    pub issues: Vec<Value>,
}
impl TaskEditor {
    pub fn draft(&self) -> TaskDraft {
        TaskDraft {
            id: Some(self.id.clone()),
            revision: Some(self.revision.clone()),
            name: self.name.clone(),
            prompt: self.prompt.clone(),
            cron: self.cron.clone(),
            timezone: self.timezone.clone(),
            once: self.once,
        }
    }
}

fn base_url(base: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(base).map_err(|_| error("Invalid Manage origin"))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error("Invalid Manage origin"));
    }
    Ok(url)
}

fn request(
    base: &str,
    token: &str,
    site: &str,
    query: &[(&str, String)],
    body: Option<Value>,
) -> Result<Value> {
    let base = base_url(base)?;
    if token.is_empty() || site.is_empty() {
        return Err(error("Select a Manage site and sign in"));
    }
    let url = base
        .join("/api/manage/shelldeck/automations")
        .map_err(|_| error("Invalid Manage endpoint"))?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| error("Cannot create Manage client"))?;
    let request = if let Some(mut body) = body {
        body["site_id"] = json!(site);
        client.post(url).json(&body)
    } else {
        client.get(url).query(&[("site_id", site)]).query(query)
    };
    let response = request
        .bearer_auth(token)
        .send()
        .map_err(|_| error("Manage request failed"))?;
    let status = response.status().as_u16();
    let value: Value = response
        .json()
        .map_err(|_| error(format!("Invalid Manage response (HTTP {status})")))?;
    if !(200..300).contains(&status) || value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(AutomationError {
            status,
            needs_reconnect: value.get("needs_reconnect").and_then(Value::as_bool) == Some(true),
            message: value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Manage request refused")
                .chars()
                .take(600)
                .collect(),
        });
    }
    Ok(value)
}

pub fn fetch(base: &str, token: &str, site: &str, page: usize) -> Result<AutomationData> {
    serde_json::from_value(request(
        base,
        token,
        site,
        &[("page", page.max(1).to_string())],
        None,
    )?)
    .map_err(|_| error("Invalid automation list"))
}
pub fn editor(base: &str, token: &str, site: &str, id: &str) -> Result<TaskEditor> {
    serde_json::from_value(
        request(base, token, site, &[("id", id.into())], None)?["editor"].clone(),
    )
    .map_err(|_| error("Invalid task details"))
}
pub fn action(base: &str, token: &str, site: &str, body: Value) -> Result<Value> {
    request(base, token, site, &[], Some(body))
}
pub fn save(base: &str, token: &str, site: &str, draft: &TaskDraft) -> Result<TaskEditor> {
    let mut body = serde_json::to_value(draft).map_err(|_| error("Invalid task draft"))?;
    body["action"] = json!("draft");
    serde_json::from_value(action(base, token, site, body)?["editor"].clone())
        .map_err(|_| error("Invalid saved task"))
}
pub fn open_url(
    base: &str,
    token: &str,
    site: &str,
    destination: &str,
    id: Option<&str>,
) -> Result<String> {
    let value = action(
        base,
        token,
        site,
        json!({ "action": "open", "destination": destination, "id": id }),
    )?;
    let url = reqwest::Url::parse(value["url"].as_str().unwrap_or_default())
        .map_err(|_| error("Invalid Manage link"))?;
    if url.origin() != base_url(base)?.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/api/manage/switch"
        || !url
            .query_pairs()
            .any(|(key, value)| key == "handoff" && !value.is_empty())
    {
        return Err(error("Invalid Manage link"));
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    // SDTEST-2932 / SDUC-572: actual request route, device bearer, site and
    // schedule fields survive the HTTP boundary; null dates are accepted.
    #[test]
    fn automation_http_contract_preserves_scope_and_schedule_fields() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 8192];
            let read = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..read]);
            assert!(request
                .starts_with("GET /api/manage/shelldeck/automations?site_id=selected&page=2 "));
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer sd_fixture\r\n"));
            let body = r#"{"ok":true,"workflows":[{"id":"w","name":"Brief","status":"ACTIVE"}],"schedules":[{"id":"s","workflowId":"w","cron":"0 9 * * 5","timezone":"Europe/Paris","isActive":true,"nextRun":null,"runCount":2}],"runs":[],"total":1}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let data = fetch(&base, "sd_fixture", "selected", 2).unwrap();
        assert_eq!(data.schedules[0].cron, "0 9 * * 5");
        assert_eq!(data.schedules[0].timezone, "Europe/Paris");
        assert_eq!(data.schedules[0].run_count, 2);
        assert!(data.schedules[0].next_run.is_none());
        worker.join().unwrap();
    }

    // SDTEST-2934 / SDUC-572: reconnect is distinct from device revocation, and
    // a signed-looking foreign URL never becomes an authenticated browser link.
    #[test]
    fn automation_errors_preserve_reconnect_and_refuse_foreign_handoffs() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for (status, body) in [
                (
                    "409 Conflict",
                    r#"{"error":"Reconnect","needs_reconnect":true}"#,
                ),
                (
                    "200 OK",
                    r#"{"ok":true,"url":"https://foreign.invalid/api/manage/switch?handoff=signed"}"#,
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = [0; 8192];
                let _ = stream.read(&mut bytes).unwrap();
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let rejected = fetch(&base, "sd_fixture", "site", 1).unwrap_err();
        assert_eq!(rejected.status, 409);
        assert!(rejected.needs_reconnect);
        assert_eq!(
            open_url(&base, "sd_fixture", "site", "templates", None)
                .unwrap_err()
                .message,
            "Invalid Manage link"
        );
        worker.join().unwrap();
    }
}
