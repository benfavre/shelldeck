//! Local dispatch memory only. It is never a scheduler or a replay queue.
use super::platform_review::PlatformReviewTarget;
use super::platform_tasks::PlatformTaskCommand;
use crate::workspace_review::storage::{
    bounded_descriptor_read, ensure_private_directory_io, lock_path, open_lock_file,
    secure_atomic_write,
};
use crate::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct TaskDispatchJournal {
    path: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u16,
    records: Vec<Record>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    key: String,
    project: String,
    workspace: String,
    tasks: Vec<String>,
    action: String,
    unresolved: bool,
}

impl TaskDispatchJournal {
    pub fn for_connection(endpoint: &str, token: &str) -> Self {
        // Token bytes never reach disk. Distinct accounts/endpoints cannot see
        // or acknowledge one another's dispatch state, even for equal task IDs.
        let mut digest = Sha256::new();
        digest.update(endpoint.as_bytes());
        digest.update([0]);
        digest.update(token.as_bytes());
        let namespace = format!("{:x}", digest.finalize());
        Self {
            path: super::AppConfig::config_dir()
                .join("platform-tasks")
                .join(namespace)
                .join("dispatch.json"),
        }
    }

    fn transaction<T>(&self, update: impl FnOnce(&mut Journal) -> Result<T>) -> Result<T> {
        let _guard = LOCK.lock();
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ShellDeckError::Config("invalid task journal path".into()))?;
        ensure_private_directory_io(parent)?;
        let lock = open_lock_file(&lock_path(&self.path))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let original = bounded_descriptor_read(&self.path, MAX_BYTES)?;
        let mut journal = match original.as_ref() {
            Some(bytes) => serde_json::from_slice::<Journal>(bytes).map_err(|_| {
                ShellDeckError::Config(
                    "task dispatch history is unreadable; preserved without replay".into(),
                )
            })?,
            None => Journal {
                version: 1,
                records: Vec::new(),
            },
        };
        if journal.version != 1 || journal.records.len() > 1024 {
            return Err(ShellDeckError::Config(
                "task dispatch history version or size is unsupported".into(),
            ));
        }
        let result = update(&mut journal)?;
        let bytes = loop {
            let bytes = serde_json::to_vec(&journal)
                .map_err(|error| ShellDeckError::Config(error.to_string()))?;
            if bytes.len() as u64 <= MAX_BYTES {
                break bytes;
            }
            let mut remaining = bytes.len() as u64;
            let mut index = 0;
            let mut removed = false;
            while remaining > MAX_BYTES && index < journal.records.len() {
                if journal.records[index].unresolved {
                    index += 1;
                    continue;
                }
                let record_bytes = serde_json::to_vec(&journal.records[index])
                    .map_err(|error| ShellDeckError::Config(error.to_string()))?;
                remaining = remaining.saturating_sub(record_bytes.len() as u64 + 1);
                journal.records.remove(index);
                removed = true;
            }
            if !removed {
                return Err(ShellDeckError::Config(
                    "task dispatch history has too many unresolved operations".into(),
                ));
            }
        };
        if original.as_deref() != Some(bytes.as_slice()) {
            secure_atomic_write(&self.path, &bytes)?;
        }
        Ok(result)
    }

    pub fn begin(&self, command: &PlatformTaskCommand) -> Result<()> {
        let target = command.target();
        let (key, action) = match command {
            PlatformTaskCommand::Create(request) => (request.idempotency_key.as_str(), "create"),
            PlatformTaskCommand::Control(request) => {
                (request.idempotency_key.as_str(), request.action.as_str())
            }
        };
        self.transaction(|journal| {
            if journal.records.iter().any(|record| record.key == key ||
                (record.unresolved && record.project == target.project.as_str() && record.workspace == target.workspace.id())) {
                return Err(ShellDeckError::Config("task dispatch already recorded or unresolved; inspect server state instead of replaying".into()));
            }
            if journal.records.len() >= 1024 {
                // This is bounded dispatch memory, not an operation replay log.
                // Never evict an unresolved dispatch; settled old entries can retire.
                let oldest_settled = journal.records.iter().position(|record| !record.unresolved)
                    .ok_or_else(|| ShellDeckError::Config("task dispatch history has too many unresolved operations".into()))?;
                journal.records.remove(oldest_settled);
            }
            journal.records.push(Record { key: key.to_owned(), project: target.project.as_str().to_owned(), workspace: target.workspace.id().to_owned(),
                tasks: command.task_ids().iter().map(|id| id.as_str().to_owned()).collect(), action: action.to_owned(), unresolved: true });
            Ok(())
        })
    }

    pub fn finish(&self, command: &PlatformTaskCommand) -> Result<()> {
        let key = match command {
            PlatformTaskCommand::Create(r) => r.idempotency_key.as_str(),
            PlatformTaskCommand::Control(r) => r.idempotency_key.as_str(),
        };
        self.transaction(|journal| {
            let record = journal
                .records
                .iter_mut()
                .find(|record| record.key == key)
                .ok_or_else(|| ShellDeckError::Config("task dispatch record missing".into()))?;
            record.unresolved = false;
            Ok(())
        })
    }

    pub fn unresolved(&self, target: &PlatformReviewTarget) -> Result<Vec<String>> {
        self.transaction(|journal| {
            Ok(journal
                .records
                .iter()
                .filter(|record| {
                    record.unresolved
                        && record.project == target.project.as_str()
                        && record.workspace == target.workspace.id()
                })
                .map(|record| format!("{}: {}", record.action, record.tasks.join(", ")))
                .collect())
        })
    }

    pub fn acknowledge(&self, target: &PlatformReviewTarget) -> Result<()> {
        self.transaction(|journal| {
            for record in &mut journal.records {
                if record.project == target.project.as_str()
                    && record.workspace == target.workspace.id()
                {
                    record.unresolved = false;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::platform_tasks::*;
    use automonique_protocol::{
        platform::IdempotencyKey,
        platform_v2::{ProjectId, UserWorkspaceId},
        primitives::Revision,
    };

    // SDTEST-2703 / SDUC-560
    #[test]
    fn sdtest_2703_restart_keeps_unknown_dispatch_lookup_only_and_acknowledgment_does_not_allow_replay(
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal/dispatch.json");
        let command = PlatformTaskCommand::Control(TaskControlRequest {
            project: ProjectId::new("project-test").unwrap(),
            workspace: UserWorkspaceId::new("workspace-test").unwrap(),
            task: OrchestrationTaskId::new("task-test").unwrap(),
            expected_revision: Revision::FIRST,
            idempotency_key: IdempotencyKey::new("request-test").unwrap(),
            action: TaskControlAction::Pause,
        });
        let journal = TaskDispatchJournal { path: path.clone() };
        journal.begin(&command).unwrap();
        let restarted = TaskDispatchJournal { path };
        assert_eq!(restarted.unresolved(&command.target()).unwrap().len(), 1);
        assert!(restarted.begin(&command).is_err());
        let mut foreign = command.target();
        foreign.project = ProjectId::new("another-project").unwrap();
        restarted.acknowledge(&foreign).unwrap();
        assert_eq!(restarted.unresolved(&command.target()).unwrap().len(), 1);
        restarted.acknowledge(&command.target()).unwrap();
        assert!(restarted.unresolved(&command.target()).unwrap().is_empty());
        assert!(
            restarted.begin(&command).is_err(),
            "acknowledgment must not permit replay"
        );
    }
    // SDTEST-2706 / SDUC-560
    #[test]
    fn sdtest_2706_settled_history_does_not_lock_out_new_controls_or_drop_unresolved_records() {
        let root = tempfile::tempdir().unwrap();
        let journal = TaskDispatchJournal {
            path: root.path().join("journal/dispatch.json"),
        };
        journal
            .transaction(|history| {
                for index in 0..1024 {
                    history.records.push(Record {
                        key: format!("key-{index}"),
                        project: "previous".into(),
                        workspace: "previous".into(),
                        tasks: vec!["old".into()],
                        action: "pause".into(),
                        unresolved: index == 0,
                    });
                }
                Ok(())
            })
            .unwrap();
        let command = PlatformTaskCommand::Control(TaskControlRequest {
            project: ProjectId::new("project-new").unwrap(),
            workspace: UserWorkspaceId::new("workspace-new").unwrap(),
            task: OrchestrationTaskId::new("task-new").unwrap(),
            expected_revision: Revision::FIRST,
            idempotency_key: IdempotencyKey::new("request-new").unwrap(),
            action: TaskControlAction::Pause,
        });
        journal.begin(&command).unwrap();
        journal
            .transaction(|history| {
                assert_eq!(history.records.len(), 1024);
                assert!(history
                    .records
                    .iter()
                    .any(|record| record.key == "key-0" && record.unresolved));
                assert!(!history.records.iter().any(|record| record.key == "key-1"));
                Ok(())
            })
            .unwrap();
        assert!(journal.begin(&command).is_err());
        journal
            .transaction(|history| {
                for record in &mut history.records {
                    record.tasks = vec!["x".repeat(2048)];
                }
                Ok(())
            })
            .unwrap();
        journal
            .transaction(|history| {
                assert!(history.records.len() < 1024);
                assert!(history
                    .records
                    .iter()
                    .any(|record| record.key == "key-0" && record.unresolved));
                assert!(history
                    .records
                    .iter()
                    .any(|record| record.key == "request-new" && record.unresolved));
                Ok(())
            })
            .unwrap();
        assert_ne!(
            TaskDispatchJournal::for_connection("endpoint", "token-a").path,
            TaskDispatchJournal::for_connection("endpoint", "token-b").path
        );
    }
}
