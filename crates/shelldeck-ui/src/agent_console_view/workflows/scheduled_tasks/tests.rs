use super::*;
// SDTEST-2936: normal scheduler admission and run UUID routing are reused;
// a run-now preview never activates a task, and context/role drift blocks it.
#[::core::prelude::v1::test]
fn native_local_tasks_reuse_session_admission_and_permission_gates() {
    let mut app = TestAppContext::single();
    let agents = app.update(|cx| cx.new(AgentConsoleView::new));
    app.run_until_parked();
    let directory = tempfile::tempdir().unwrap();
    agents.update(&mut app, |agents, cx| {
        agents.workflows.store =
            AgentWorkflowStore::new(directory.path().join("agents/workflows/tasks.json"));
        agents.workflows.state = AgentWorkflows::default();
        agents.workflows.writable = true;
        agents.workflows.loading = false;
        agents.set_workflows_authorized(true, cx);
        let context = AgentExecutionContext {
            provider: AgentProvider::Codex,
            target: AgentTarget::Local,
            access: AgentAccessMode::ReadOnly,
            workdir: directory.path().to_string_lossy().into_owned(),
            model: None,
        };
        let session = agents
            .create_session("Scheduled local briefing", context.clone(), cx)
            .unwrap();
        let calendar = CalendarSchedule {
            cron: "0 9 * * *".into(),
            timezone: "Europe/Paris".into(),
            once: false,
        };
        let id = agents
            .save_local_scheduled_task(
                None,
                session,
                "Daily brief".into(),
                "Read the sources".into(),
                calendar,
                cx,
            )
            .unwrap();
        let next = agents.workflows.state.schedules[0].next_due_ms;
        assert!(!agents.workflows.state.schedules[0].enabled);
        agents
            .local_task_command(id, LocalTaskCommand::RunNow, cx)
            .unwrap();
        let run = agents
            .sessions
            .get(session)
            .unwrap()
            .turns
            .last()
            .unwrap()
            .id;
        assert_eq!(agents.session_id_for_run(run), Some(session));
        assert_eq!(agents.sessions.get(session).unwrap().context, context);
        assert_eq!(agents.workflows.state.schedules[0].next_due_ms, next);
        assert!(!agents.workflows.state.schedules[0].enabled);
        agents.finish_run(run, Some(0), None, cx);
        agents
            .local_task_command(id, LocalTaskCommand::Activate, cx)
            .unwrap();
        agents.workflows.state.schedules[0].next_due_ms = now_ms() - 1;
        agents.poll_agent_workflows(cx);
        let scheduled = agents.workflows.state.schedules[0].active_run.unwrap();
        assert_ne!(scheduled, run);
        assert_eq!(agents.session_id_for_run(scheduled), Some(session));
        agents.finish_run(scheduled, Some(0), None, cx);
        agents.change_draft_access(AgentAccessMode::WorkspaceWrite, cx);
        assert!(agents
            .local_task_command(id, LocalTaskCommand::RunNow, cx)
            .is_err());
        agents.workflows.state.schedules[0].next_due_ms = now_ms() - 1;
        agents.poll_agent_workflows(cx);
        assert!(!agents.workflows.state.schedules[0].enabled);
        assert_eq!(
            agents.workflows.state.schedules[0].pause_reason,
            Some(WorkflowPauseReason::ContextChanged)
        );
        agents
            .local_task_command(id, LocalTaskCommand::Pause, cx)
            .unwrap();
        agents.set_workflows_authorized(false, cx);
        assert!(agents
            .local_task_command(id, LocalTaskCommand::Activate, cx)
            .is_err());
        assert!(agents.local_scheduled_tasks().is_empty());
    });
    app.run_until_parked();
    let loaded = AgentWorkflowStore::new(directory.path().join("agents/workflows/tasks.json"))
        .load()
        .unwrap();
    assert_eq!(loaded.schedules.len(), 1);
    assert!(loaded.schedules[0].calendar.is_some());
    assert!(!loaded.schedules[0].enabled);
}
