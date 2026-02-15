use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub id: Uuid,
    pub script_id: Uuid,
    pub connection_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub output_log: String,
}

impl ExecutionRecord {
    pub fn new(script_id: Uuid, connection_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            script_id,
            connection_id,
            started_at: Utc::now(),
            finished_at: None,
            exit_code: None,
            output_log: String::new(),
        }
    }

    pub fn finish(&mut self, exit_code: i32) {
        self.finished_at = Some(Utc::now());
        self.exit_code = Some(exit_code);
    }

    pub fn append_output(&mut self, data: &str) {
        self.output_log.push_str(data);
    }

    pub fn duration_secs(&self) -> Option<f64> {
        self.finished_at
            .map(|end| (end - self.started_at).num_milliseconds() as f64 / 1000.0)
    }

    pub fn is_running(&self) -> bool {
        self.finished_at.is_none()
    }

    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}
