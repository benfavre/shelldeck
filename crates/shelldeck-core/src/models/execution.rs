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

#[cfg(test)]
mod tests {
    use super::ExecutionRecord;
    use uuid::Uuid;

    // SDTEST-044 — new → running, no exit code, empty log, no duration.
    #[test]
    fn new_starts_in_running_state() {
        let r = ExecutionRecord::new(Uuid::new_v4(), None);
        assert!(r.is_running());
        assert!(!r.succeeded(), "no exit code yet ⇒ not succeeded");
        assert!(r.exit_code.is_none());
        assert!(r.finished_at.is_none());
        assert!(r.duration_secs().is_none());
        assert!(r.output_log.is_empty());
    }

    // append_output accumulates without allocating a fresh String each
    // call (`push_str` in place).
    #[test]
    fn append_output_accumulates() {
        let mut r = ExecutionRecord::new(Uuid::new_v4(), None);
        r.append_output("line 1\n");
        r.append_output("line 2\n");
        r.append_output("");
        assert_eq!(r.output_log, "line 1\nline 2\n");
    }

    // finish(0) → succeeded, duration observable and non-negative.
    #[test]
    fn finish_with_zero_marks_succeeded_and_produces_duration() {
        let mut r = ExecutionRecord::new(Uuid::new_v4(), None);
        // Wait a hair so `duration_secs > 0` is actually observable at
        // millisecond precision. 5ms is plenty and keeps the test fast.
        std::thread::sleep(std::time::Duration::from_millis(5));
        r.finish(0);
        assert!(!r.is_running());
        assert!(r.succeeded());
        let d = r.duration_secs().expect("finish set finished_at");
        assert!(d >= 0.0, "duration must be non-negative, got {d}");
    }

    // Any non-zero exit code ⇒ not succeeded.
    #[test]
    fn finish_with_non_zero_marks_failure() {
        for code in [1, 2, 127, -1, 255] {
            let mut r = ExecutionRecord::new(Uuid::new_v4(), None);
            r.finish(code);
            assert!(!r.is_running());
            assert!(!r.succeeded(), "exit={code}: should not be succeeded");
            assert_eq!(r.exit_code, Some(code));
        }
    }

    // connection_id round-trips: None for a local script, Some(uuid)
    // for a remote script. Consumed by the sidebar activity feed to
    // route entries to the right connection row.
    #[test]
    fn connection_id_is_preserved() {
        let conn = Uuid::new_v4();
        let local = ExecutionRecord::new(Uuid::new_v4(), None);
        let remote = ExecutionRecord::new(Uuid::new_v4(), Some(conn));
        assert!(local.connection_id.is_none());
        assert_eq!(remote.connection_id, Some(conn));
    }
}
