use super::*;
use shelldeck_core::agent_codex::{spawn_codex_run, LiveCodexCommand, LiveCodexOutcome};
use shelldeck_core::agent_runtime::{AgentRunRequest, AgentStreamEvent};

impl Workspace {
    pub(super) fn start_live_codex(&mut self, request: AgentRunRequest, cx: &mut Context<Self>) {
        let run_id = request.id;
        let Some(program) = super::agents::resolve_local_agent_program("codex") else {
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(
                    run_id,
                    t!("agents.error.binary_missing", binary = "codex").to_string(),
                    cx,
                )
            });
            return;
        };
        let mut options = self
            .agent_console
            .update(cx, |view, _| view.take_run_options(run_id));
        if !std::path::Path::new(&request.workdir).is_dir() {
            self.agent_console.update(cx, |view, cx| {
                view.reject_run_for(
                    run_id,
                    t!(
                        "agents.error.workdir_missing",
                        workdir = request.workdir.clone()
                    )
                    .to_string(),
                    cx,
                )
            });
            return;
        }
        if let Err(error) = self.bind_agent_run_to_workspace(&request, cx) {
            self.agent_console
                .update(cx, |view, cx| view.reject_run_for(run_id, error, cx));
            return;
        }
        if let Err(error) = self.agent_console.update(cx, |view, cx| {
            view.attach_browser_tools(&request, &mut options, cx)
        }) {
            self.agent_console
                .update(cx, |view, cx| view.reject_run_for(run_id, error, cx));
            return;
        }
        self.agent_console
            .update(cx, |view, cx| view.begin_run(request.clone(), cx));
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentStreamEvent>();
        let live = match spawn_codex_run(
            request,
            options,
            std::path::PathBuf::from(program),
            event_tx,
        ) {
            Ok(live) => live,
            Err(error) => {
                self.agent_console.update(cx, |view, cx| {
                    view.finish_run(run_id, None, Some(error.to_string()), cx)
                });
                return;
            }
        };
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let commands = live.commands;
        let cancel = live.cancel;
        let thread = std::thread::spawn(move || {
            let result = match live.join.join() {
                Ok(Ok(LiveCodexOutcome::Completed)) => (Some(0), None),
                Ok(Ok(LiveCodexOutcome::Interrupted)) => (None, None),
                Ok(Err(error)) => (None, Some(error.to_string())),
                Err(_) => (None, Some("Codex worker stopped unexpectedly".to_string())),
            };
            let _ = done_tx.send(result);
        });
        self.active_agent_runs.insert(
            run_id,
            super::agents::ActiveAgentRun::live_codex(commands, cancel, thread),
        );
        let console = self.agent_console.downgrade();
        let mut pump = super::agent_stream::AgentEventPump::new(event_rx, done_rx);
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(super::agent_stream::FRAME_INTERVAL)
                .await;
            let (events, completion) = pump.poll();
            if !events.is_empty() {
                let _ = console.update(cx, |view, cx| {
                    view.push_stream_events_for(run_id, events, cx)
                });
            }
            if let Some((exit_code, error)) = completion {
                let _ = this.update(cx, |workspace, cx| {
                    workspace.complete_agent_run(run_id, exit_code, error, cx)
                });
                break;
            }
        })
        .detach();
    }

    pub(super) fn send_live_codex(
        &mut self,
        run_id: Uuid,
        command: LiveCodexCommand,
        cx: &mut Context<Self>,
    ) {
        let user_input = matches!(
            &command,
            LiveCodexCommand::Steer { .. } | LiveCodexCommand::Answer { .. }
        );
        let result = self
            .active_agent_runs
            .get(&run_id)
            .ok_or("Agent run is no longer active".to_string())
            .and_then(|run| run.send_live(command));
        if let Err(error) = result {
            if user_input {
                self.agent_console.update(cx, |view, cx| {
                    view.reject_live_input(run_id, error.clone(), cx)
                });
            }
            self.show_toast(error, crate::toast::ToastLevel::Warning, cx);
        }
    }
}
