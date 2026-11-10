//! Calendar recurrence shared by the desktop task editor and existing scheduler.
use super::*;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSchedule {
    pub cron: String,
    pub timezone: String,
    pub once: bool,
}
impl CalendarSchedule {
    fn parsed(&self) -> Result<(croner::Cron, chrono_tz::Tz)> {
        if self.cron.len() > 256
            || self.cron.split_whitespace().count() != 5
            || self.timezone.len() > 128
        {
            return Err(invalid("expected a five-field cron and an IANA timezone"));
        }
        let cron = croner::Cron::from_str(&self.cron)
            .map_err(|_| invalid("invalid calendar recurrence"))?;
        let timezone = self
            .timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| invalid("invalid IANA timezone"))?;
        Ok((cron, timezone))
    }
    pub fn validate(&self) -> Result<()> {
        self.parsed().map(|_| ())
    }
    /// Strictly future, in the selected timezone. Missed ticks coalesce in
    /// AgentSchedule::claim_due; no replay loop or provider process lives here.
    pub fn next_after(&self, now_ms: i64) -> Result<i64> {
        let (cron, timezone) = self.parsed()?;
        let now = chrono::DateTime::from_timestamp_millis(now_ms)
            .ok_or_else(|| invalid("invalid schedule timestamp"))?
            .with_timezone(&timezone);
        cron.find_next_occurrence(&now, false)
            .map(|next| next.timestamp_millis())
            .map_err(|_| invalid("calendar recurrence has no next occurrence"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(value: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }
    // SDTEST-2935: calendar time survives DST; missed ticks coalesce; a
    // one-shot is not replayed after restart, and legacy intervals still load.
    #[test]
    fn calendar_tasks_keep_wall_time_and_revalidate_dispatch() {
        let calendar = CalendarSchedule {
            cron: "0 9 * * *".into(),
            timezone: "Europe/Paris".into(),
            once: false,
        };
        assert_eq!(
            calendar
                .next_after(ms("2027-03-27T09:00:00+01:00"))
                .unwrap(),
            ms("2027-03-28T09:00:00+02:00")
        );
        assert_eq!(
            calendar
                .next_after(ms("2027-10-30T09:00:00+02:00"))
                .unwrap(),
            ms("2027-10-31T09:00:00+01:00")
        );
        let context = AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: "/tmp".into(),
            model: None,
        };
        let mut task = AgentSchedule::new_calendar(
            Uuid::new_v4(),
            context,
            "Brief".into(),
            "Read sources".into(),
            calendar.clone(),
            ms("2027-03-20T10:00:00Z"),
        )
        .unwrap();
        task.enable(ms("2027-03-20T10:00:00Z"));
        let now = ms("2027-03-28T10:00:00Z");
        assert!(task.claim_due(now, |_, _| true).is_some());
        assert_eq!(task.next_due_ms, ms("2027-03-29T09:00:00+02:00"));
        assert!(task.claim_due(now, |_, _| true).is_none());
        assert!(task.claim_due(task.next_due_ms, |_, _| false).is_none());
        assert!(!task.enabled);

        task.calendar.as_mut().unwrap().once = true;
        task.last_run_ms = None;
        task.enable(now);
        assert!(task.claim_due(task.next_due_ms, |_, _| true).is_some());
        assert_eq!(task.next_due_ms, i64::MAX);
        let mut state = AgentWorkflows {
            schedules: vec![task.clone()],
            ..Default::default()
        };
        state.recover();
        assert!(!state.schedules[0].enabled);
        assert!(state.schedules[0].claim_due(now, |_, _| true).is_none());

        let mut value = serde_json::to_value(task).unwrap();
        value.as_object_mut().unwrap().remove("calendar");
        let legacy: AgentSchedule = serde_json::from_value(value).unwrap();
        assert!(legacy.calendar.is_none());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agents/workflows/tasks.json");
        let store = AgentWorkflowStore::new(&path);
        store.save(&state).unwrap();
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            envelope["schema_version"], 2,
            "Legacy readers must refuse calendar tasks"
        );
        assert!(store.load().unwrap().schedules[0].calendar.is_some());
        envelope["schema_version"] = serde_json::json!(1);
        std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(
            store.load().is_err(),
            "Calendar intent cannot masquerade as the legacy interval format"
        );
        store
            .save(&AgentWorkflows {
                schedules: vec![legacy],
                ..Default::default()
            })
            .unwrap();
        let envelope: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(envelope["schema_version"], 1);
        assert!(store.load().unwrap().schedules[0].calendar.is_none());
        assert!(CalendarSchedule {
            cron: "61 * * * *".into(),
            ..calendar.clone()
        }
        .next_after(now)
        .is_err());
        assert!(CalendarSchedule {
            timezone: "not/a-zone".into(),
            ..calendar
        }
        .next_after(now)
        .is_err());
    }
}
