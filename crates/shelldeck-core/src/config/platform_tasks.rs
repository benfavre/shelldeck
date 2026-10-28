//! Presentation and explicit requests for server-owned durable tasks.
//!
//! ShellDeck neither schedules these tasks nor accepts agent claims as evidence.
//! A lost mutation response is an unknown outcome: subsequent requests read state
//! and never silently replay the mutation.

use automonique_platform_client::platform_v2_client::{
    NegotiationResult, PlatformV2Client, TaskListResult, TaskReadResult,
};
use automonique_protocol::platform_v2::{
    PlatformVersionOffer, UserWorkspaceId, WorkContextIdentity,
};
pub use automonique_protocol::platform_v2_lineage::{OrchestrationRunId, OrchestrationTaskId};
pub use automonique_protocol::platform_v2_tasks::{
    TaskCheck, TaskControlAction, TaskControlRequest, TaskCreateRequest, TaskCriterion,
    TaskEvidence, TaskPlan, TaskSnapshot, TaskSpec, TaskStatus,
};

use super::platform_review::PlatformReviewTarget;
use crate::{Result, ShellDeckError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformTaskLoad {
    Available(Vec<TaskSnapshot>, Vec<String>),
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformTaskCommand {
    Create(TaskCreateRequest),
    Control(TaskControlRequest),
}

impl PlatformTaskCommand {
    pub fn target(&self) -> PlatformReviewTarget {
        let (project, workspace) = match self {
            Self::Create(request) => (&request.project, &request.workspace),
            Self::Control(request) => (&request.project, &request.workspace),
        };
        PlatformReviewTarget {
            project: project.clone(),
            workspace: WorkContextIdentity::UserWorkspace(workspace.clone()),
        }
    }

    pub fn task_ids(&self) -> Vec<OrchestrationTaskId> {
        match self {
            Self::Create(request) => request
                .plan
                .tasks
                .iter()
                .map(|task| task.id.clone())
                .collect(),
            Self::Control(request) => vec![request.task.clone()],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformTaskCommandResult {
    Applied(Vec<TaskSnapshot>),
    Refused(String),
    /// Dispatch may have reached the server. Read state; never replay automatically.
    Unknown(String),
}

fn connection_error(message: impl Into<String>) -> ShellDeckError {
    ShellDeckError::Connection(message.into())
}

pub fn task_workspace(target: &PlatformReviewTarget) -> Result<UserWorkspaceId> {
    match &target.workspace {
        WorkContextIdentity::UserWorkspace(workspace) => Ok(workspace.clone()),
        _ => Err(connection_error(
            "durable tasks require an exact user workspace",
        )),
    }
}

fn negotiate<T>(client: &mut PlatformV2Client<T>) -> Result<()> {
    let offer = PlatformVersionOffer::new(vec![2])
        .map_err(|_| connection_error("invalid platform offer"))?;
    match client
        .negotiate(offer)
        .map_err(|error| connection_error(error.category()))?
    {
        NegotiationResult::V2(_) => Ok(()),
        NegotiationResult::Downgraded(_) => {
            Err(connection_error("durable tasks require Platform v2"))
        }
        NegotiationResult::Refused(refusal) => {
            Err(connection_error(refusal.explanation().as_str()))
        }
    }
}

pub fn validate_task_scope(target: &PlatformReviewTarget, tasks: &[TaskSnapshot]) -> Result<()> {
    let workspace = task_workspace(target)?;
    let mut seen = std::collections::BTreeSet::new();
    if tasks.iter().any(|task| {
        task.project != target.project
            || task.workspace != workspace
            || !seen.insert(task.spec.id.clone())
    }) {
        return Err(connection_error(
            "task response does not match the requested workspace",
        ));
    }
    Ok(())
}

pub(super) fn load_tasks<T>(
    client: &mut PlatformV2Client<T>,
    target: &PlatformReviewTarget,
) -> Result<PlatformTaskLoad> {
    let workspace = task_workspace(target)?;
    negotiate(client)?;
    match client
        .list_tasks(target.project.clone(), workspace)
        .map_err(|error| connection_error(error.category()))?
    {
        TaskListResult::Tasks(tasks) => {
            validate_task_scope(target, &tasks)?;
            Ok(PlatformTaskLoad::Available(tasks, Vec::new()))
        }
        TaskListResult::Refused(refusal) => Ok(PlatformTaskLoad::Unavailable(
            refusal.explanation().as_str().to_owned(),
        )),
    }
}

fn command_refusal(
    refusal: automonique_protocol::platform_v2_transport::PlatformV2Refusal,
) -> PlatformTaskCommandResult {
    let explanation = refusal.explanation().as_str().to_owned();
    // Older gateways may encode an indeterminate forwarded request as a refusal.
    // Unavailability cannot establish that a dispatched mutation was rejected.
    if refusal.category().as_str() == "platform_node_unavailable" {
        PlatformTaskCommandResult::Unknown(explanation)
    } else {
        PlatformTaskCommandResult::Refused(explanation)
    }
}

pub(super) fn execute_task_command<T>(
    client: &mut PlatformV2Client<T>,
    command: &PlatformTaskCommand,
) -> PlatformTaskCommandResult {
    if let Err(error) = negotiate(client) {
        // No task request has been sent when negotiation fails.
        return PlatformTaskCommandResult::Refused(error.to_string());
    }
    let response = match command {
        PlatformTaskCommand::Create(request) => {
            client
                .create_tasks(request.clone())
                .map(|result| match result {
                    TaskListResult::Tasks(tasks) => PlatformTaskCommandResult::Applied(tasks),
                    TaskListResult::Refused(refusal) => command_refusal(refusal),
                })
        }
        PlatformTaskCommand::Control(request) => {
            client
                .control_task(request.clone())
                .map(|result| match result {
                    TaskReadResult::Task(task) => PlatformTaskCommandResult::Applied(vec![task]),
                    TaskReadResult::Refused(refusal) => command_refusal(refusal),
                })
        }
    };
    match response {
        Ok(PlatformTaskCommandResult::Applied(tasks)) => {
            let expected: std::collections::BTreeSet<_> = command.task_ids().into_iter().collect();
            let observed: std::collections::BTreeSet<_> =
                tasks.iter().map(|task| task.spec.id.clone()).collect();
            if validate_task_scope(&command.target(), &tasks).is_err() || expected != observed {
                PlatformTaskCommandResult::Unknown(
                    "task response identity mismatch; refresh server state".to_owned(),
                )
            } else {
                PlatformTaskCommandResult::Applied(tasks)
            }
        }
        Ok(result) => result,
        Err(error) => PlatformTaskCommandResult::Unknown(error.category().to_owned()),
    }
}

/// "Verified" requires current evidence for every criterion, even if an
/// inconsistent server labels a task verified. Agent prose is never inspected.
pub fn task_has_current_verification(task: &TaskSnapshot) -> bool {
    task.validate().is_ok()
        && task.status == TaskStatus::Verified
        && !task.spec.criteria.is_empty()
        && task.attempt_id.is_some()
        && task.artifact_revision.is_some()
        && task.spec.criteria.iter().all(|criterion| {
            let matching: Vec<_> = task
                .evidence
                .iter()
                .filter(|evidence| {
                    evidence.criterion_id == criterion.id
                        && Some(&evidence.attempt_id) == task.attempt_id.as_ref()
                        && Some(&evidence.artifact_revision) == task.artifact_revision.as_ref()
                })
                .collect();
            matching.len() == 1 && matching[0].passed
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use automonique_platform_client::platform_v2_client::testing::{
        DeterministicPlatformV2Step as Step, DeterministicPlatformV2Transport,
    };
    use automonique_protocol::{
        platform::IdempotencyKey,
        platform_v2::{NegotiatedPlatform, PlatformVersion, ProjectId, WorkContextAvailability},
        platform_v2_transport::{
            PlatformNegotiationResponse, PlatformV2Request, PlatformV2Response,
        },
        primitives::Revision,
    };

    fn fixture() -> TaskSnapshot {
        TaskSnapshot {
            project: ProjectId::new("project-fixture").unwrap(),
            workspace: UserWorkspaceId::new("workspace-fixture").unwrap(),
            run_id: OrchestrationRunId::new("run-fixture").unwrap(),
            spec: TaskSpec {
                id: OrchestrationTaskId::new("task-fixture").unwrap(),
                objective: "Write an answer artifact".into(),
                criteria: vec![TaskCriterion {
                    id: "answer".into(),
                    description: "Answer contains the required result".into(),
                    check: TaskCheck::ArtifactContains {
                        path: "answer.txt".into(),
                        text: "42".into(),
                    },
                }],
                dependencies: vec![],
                max_attempts: 2,
                attempt_timeout_ms: 60_000,
            },
            status: TaskStatus::Verified,
            revision: Revision::new(4).unwrap(),
            attempts_used: 1,
            attempt_id: Some("attempt-1".into()),
            artifact_revision: Some("a".repeat(64)),
            evidence: vec![TaskEvidence {
                criterion_id: "answer".into(),
                attempt_id: "attempt-1".into(),
                artifact_revision: "a".repeat(64),
                source: "runtime".into(),
                passed: true,
                summary: "Expected content observed".into(),
                observed_at_ms: 1,
            }],
            reason: None,
        }
    }

    fn target(task: &TaskSnapshot) -> PlatformReviewTarget {
        PlatformReviewTarget {
            project: task.project.clone(),
            workspace: WorkContextIdentity::UserWorkspace(task.workspace.clone()),
        }
    }

    fn negotiated() -> Step {
        Step::Negotiation(PlatformNegotiationResponse::Negotiated(
            NegotiatedPlatform::new(
                PlatformVersion::V2,
                PlatformVersion::V2.schema(),
                WorkContextAvailability::V2Structured,
            )
            .unwrap(),
        ))
    }

    // SDTEST-2700 / SDUC-560
    #[test]
    fn sdtest_2700_task_reads_are_scoped_and_cannot_import_another_workspaces_tasks() {
        let task = fixture();
        let scope = target(&task);
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            negotiated(),
            Step::V2(Box::new(PlatformV2Response::TaskList(vec![task.clone()]))),
        ]));
        let PlatformTaskLoad::Available(tasks, _) = load_tasks(&mut client, &scope).unwrap() else {
            panic!("expected tasks")
        };
        assert!(task_has_current_verification(&tasks[0]));
        assert!(
            matches!(client.transport().requests()[0].request(), PlatformV2Request::ListTasks(request) if request.project == scope.project && request.workspace == task.workspace)
        );
        let mut foreign = task.clone();
        foreign.project = ProjectId::new("other-project").unwrap();
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            negotiated(),
            Step::V2(Box::new(PlatformV2Response::TaskList(vec![foreign]))),
        ]));
        assert!(load_tasks(&mut client, &scope).is_err());
        assert!(
            validate_task_scope(&scope, &[task.clone(), task]).is_err(),
            "duplicate task identities must be refused"
        );
    }

    // SDTEST-2701 / SDUC-560
    #[test]
    fn sdtest_2701_verification_requires_every_current_attempt_and_artifact_check() {
        let valid = fixture();
        assert!(task_has_current_verification(&valid));
        let mut stale = valid.clone();
        stale.artifact_revision = Some("b".repeat(64));
        assert!(!task_has_current_verification(&stale));
        stale = valid.clone();
        stale.attempt_id = Some("attempt-2".into());
        assert!(!task_has_current_verification(&stale));
        stale = valid.clone();
        stale.evidence[0].passed = false;
        assert!(!task_has_current_verification(&stale));
        stale = valid.clone();
        stale.evidence.clear();
        assert!(!task_has_current_verification(&stale));
        stale = valid.clone();
        stale.status = TaskStatus::AwaitingVerification;
        assert!(!task_has_current_verification(&stale));
        stale = valid.clone();
        stale.evidence.push(valid.evidence[0].clone());
        assert!(!task_has_current_verification(&stale));
    }

    // SDTEST-2702 / SDUC-560
    #[test]
    fn sdtest_2702_ambiguous_controls_never_retry_and_failed_negotiation_never_dispatches() {
        let task = fixture();
        let command = PlatformTaskCommand::Control(TaskControlRequest {
            project: task.project,
            workspace: task.workspace,
            task: task.spec.id,
            expected_revision: task.revision,
            idempotency_key: IdempotencyKey::new("request-control").unwrap(),
            action: TaskControlAction::Pause,
        });
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            negotiated(),
            Step::MalformedResponse,
        ]));
        assert!(matches!(
            execute_task_command(&mut client, &command),
            PlatformTaskCommandResult::Unknown(_)
        ));
        assert_eq!(
            client.transport().pending_steps(),
            0,
            "only negotiation and one dispatch are attempted"
        );
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            Step::MalformedResponse,
        ]));
        assert!(matches!(
            execute_task_command(&mut client, &command),
            PlatformTaskCommandResult::Refused(_)
        ));
        assert!(
            client.transport().requests().is_empty(),
            "a failed negotiation must not send a task control"
        );
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            negotiated(),
            Step::V2(Box::new(PlatformV2Response::Refused(
                automonique_protocol::platform_v2_transport::PlatformV2Refusal::new(
                    "platform_node_unavailable",
                    "Forwarded response was lost",
                )
                .unwrap(),
            ))),
        ]));
        assert!(matches!(
            execute_task_command(&mut client, &command),
            PlatformTaskCommandResult::Unknown(_)
        ));
        assert_eq!(client.transport().requests().len(), 1);
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            negotiated(),
            Step::Error(
                automonique_platform_client::platform_v2_client::PlatformV2ClientError::UnexpectedStatus,
            ),
        ]));
        assert!(matches!(
            execute_task_command(&mut client, &command),
            PlatformTaskCommandResult::Unknown(_)
        ));
        assert_eq!(client.transport().pending_steps(), 0);
        let old_version = NegotiatedPlatform::new(
            PlatformVersion::V1,
            PlatformVersion::V1.schema(),
            WorkContextAvailability::V1ExistingResourcesOnly,
        )
        .unwrap();
        let mut client = PlatformV2Client::new_testing(DeterministicPlatformV2Transport::new([
            Step::Negotiation(PlatformNegotiationResponse::Negotiated(old_version)),
        ]));
        assert!(matches!(
            execute_task_command(&mut client, &command),
            PlatformTaskCommandResult::Refused(_)
        ));
        assert!(
            client.transport().requests().is_empty(),
            "v1 cannot dispatch a task request"
        );
    }
}
