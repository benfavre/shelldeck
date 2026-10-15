//! Fake-pipe integration tests: no real Codex process or inference.
use super::*;
use std::sync::mpsc::RecvTimeoutError;

struct FakeWire {
    incoming: Receiver<Value>,
    outgoing: Sender<Value>,
}
impl CodexWire for FakeWire {
    fn send(&mut self, value: Value) -> Result<()> {
        self.outgoing
            .send(value)
            .map_err(|_| failure("fake pipe closed"))
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<Value>> {
        match self.incoming.recv_timeout(timeout) {
            Ok(value) => Ok(Some(value)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(failure("fake pipe closed")),
        }
    }
}
struct Script {
    request: AgentRunRequest,
    options: AgentInputOptions,
    incoming: Sender<Value>,
    outgoing: Receiver<Value>,
    commands: SyncSender<LiveCodexCommand>,
    cancel: Arc<AtomicBool>,
    events: Receiver<AgentStreamEvent>,
    join: JoinHandle<Result<LiveCodexOutcome>>,
}
impl Script {
    fn new(access: AgentAccessMode, resume: Option<&str>) -> Self {
        Self::with_browser(access, resume, Vec::new())
    }
    fn with_browser(
        access: AgentAccessMode,
        resume: Option<&str>,
        mcp_servers: Vec<crate::agent_browser_tools::AgentMcpServerLaunch>,
    ) -> Self {
        let workdir = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut request = AgentRunRequest::new(
            AgentProvider::Codex,
            AgentTarget::Local,
            access,
            workdir,
            Some("chosen-model".into()),
            "Fix the selected project",
        );
        request.resume_session = resume.map(str::to_string);
        let options = AgentInputOptions {
            mcp_servers,
            reasoning_effort: Some(AgentReasoningEffort::High),
            image_paths: vec![std::env::temp_dir().join("selected screenshot.png")],
            ..Default::default()
        };
        let (incoming, in_rx) = mpsc::channel();
        let (out_tx, outgoing) = mpsc::channel();
        let (commands, cmd_rx) = mpsc::sync_channel(32);
        let (ev_tx, events) = mpsc::channel();
        let req = request.clone();
        let opts = options.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let join = std::thread::spawn(move || {
            let mut wire = FakeWire {
                incoming: in_rx,
                outgoing: out_tx,
            };
            drive(&req, &opts, &mut wire, &cmd_rx, &ev_tx, &worker_cancel)
        });
        Self {
            request,
            options,
            incoming,
            outgoing,
            commands,
            cancel,
            events,
            join,
        }
    }
    fn sent(&self) -> Value {
        self.outgoing
            .recv_timeout(Duration::from_secs(2))
            .expect("expected client request")
    }
    fn event(&self, predicate: impl Fn(&AgentStreamEvent) -> bool) -> AgentStreamEvent {
        let deadline = Instant::now() + Duration::from_secs(2);
        while let Ok(event) = self
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            if predicate(&event) {
                return event;
            }
        }
        panic!("expected presentation event")
    }
    fn respond(&self, request: &Value, result: Value) {
        self.incoming
            .send(json!({"id":request["id"],"result":result}))
            .unwrap();
    }
    fn notify(&self, method: &str, params: Value) {
        self.incoming
            .send(json!({"method":method,"params":params}))
            .unwrap();
    }
    fn server(&self, id: Value, method: &str, extra: Value) {
        let mut params =
            json!({"threadId":"thread-1","turnId":"turn-1","itemId":"tool-1","startedAtMs":0});
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.incoming
            .send(json!({"id":id,"method":method,"params":params}))
            .unwrap();
    }
    fn handshake(&self) -> Value {
        let init = self.sent();
        assert_eq!(init["method"], "initialize");
        assert_eq!(init["params"]["clientInfo"]["name"], "shelldeck");
        assert_eq!(init["params"]["capabilities"]["experimentalApi"], true);
        assert!(init.to_string().find(&self.request.prompt).is_none());
        self.respond(&init, json!({"userAgent":"fake"}));
        assert_eq!(self.sent(), json!({"method":"initialized"}));
        let thread = self.sent();
        assert_eq!(
            thread["method"],
            if self.request.resume_session.is_some() {
                "thread/resume"
            } else {
                "thread/start"
            }
        );
        assert_eq!(thread["params"]["cwd"], self.request.workdir);
        assert_eq!(thread["params"]["model"], "chosen-model");
        assert_eq!(
            thread["params"]["approvalPolicy"],
            approval_policy(self.request.access)
        );
        assert_eq!(thread["params"]["approvalsReviewer"], "user");
        if self.options.mcp_servers.is_empty() {
            assert!(thread["params"].get("config").is_none());
        } else {
            assert_eq!(
                thread["params"]["config"],
                crate::agent_browser_tools::codex_browser_config(&self.options.mcp_servers)
            );
        }
        if self.request.resume_session.is_some() {
            assert_eq!(thread["params"]["threadId"], "thread-1");
        }
        self.respond(&thread,json!({"thread":{"id":"thread-1","cliVersion":"test-version"},
            "cwd":self.request.workdir,"sandbox":sandbox(self.request.access),"model":"resolved-model"}));
        let turn = self.sent();
        assert_eq!(turn["method"], "turn/start");
        assert_eq!(turn["params"]["threadId"], "thread-1");
        assert_eq!(turn["params"]["effort"], "high");
        assert_eq!(
            turn["params"]["sandboxPolicy"],
            sandbox(self.request.access)
        );
        assert_eq!(
            turn["params"]["input"],
            json!([
            {"type":"text","text":self.request.prompt},
            {"type":"localImage","path":self.options.image_paths[0]}])
        );
        turn
    }
    fn start(&self) {
        let turn = self.handshake();
        // Real app-server may notify before acknowledging the turn request.
        self.notify(
            "turn/started",
            json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"inProgress","items":[]}}),
        );
        self.respond(
            &turn,
            json!({"turn":{"id":"turn-1","status":"inProgress","items":[]}}),
        );
        self.event(|event| matches!(event, AgentStreamEvent::Ready));
    }
    fn complete(&self, status: &str) {
        self.notify(
            "turn/completed",
            json!({"threadId":"thread-1","turn":{"id":"turn-1","status":status,"items":[]}}),
        );
    }
}

// SDTEST-2360 → SDUC-554
#[test]
fn sdtest_2360_codex_fake_pipe_steers_answers_questions_and_approves_only_exact_one_shot_requests()
{
    let script = Script::new(AgentAccessMode::WorkspaceWrite, None);
    script.start();
    script
        .commands
        .send(LiveCodexCommand::Steer {
            text: "Keep the original scope".into(),
        })
        .unwrap();
    let steer = script.sent();
    assert_eq!(steer["method"], "turn/steer");
    assert_eq!(
        steer["params"],
        json!({"threadId":"thread-1","expectedTurnId":"turn-1",
        "input":[{"type":"text","text":"Keep the original scope"}]})
    );
    script.respond(&steer, json!({"turnId":"turn-1"}));
    assert!(
        matches!(script.event(|event|matches!(event,AgentStreamEvent::UserInputAccepted {..})),
        AgentStreamEvent::UserInputAccepted {text} if text=="Keep the original scope")
    );
    script.server(json!("approval-55"),"item/commandExecution/requestApproval",json!({
        "command":"cargo test","cwd":script.request.workdir,"reason":"Run verification",
        "availableDecisions":["accept","acceptForSession","decline",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo"]}}]}));
    let AgentStreamEvent::Approval(approval) =
        script.event(|e| matches!(e, AgentStreamEvent::Approval(_)))
    else {
        unreachable!()
    };
    assert!(approval.title.contains("cargo test"));
    assert!(approval.title.contains("Run verification"));
    assert_eq!(
        approval
            .options
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        ["accept", "decline"]
    );
    assert_eq!(approval.session_id, "thread-1");
    assert_eq!(approval.tool_call_id, "tool-1");
    script
        .commands
        .send(LiveCodexCommand::Approval {
            request_id: approval.id.clone(),
            option_id: Some("acceptForSession".into()),
        })
        .unwrap();
    script.event(|e| matches!(e, AgentStreamEvent::Error(_)));
    assert!(script.outgoing.try_recv().is_err());
    script
        .commands
        .send(LiveCodexCommand::Approval {
            request_id: approval.id.clone(),
            option_id: Some("accept".into()),
        })
        .unwrap();
    assert_eq!(
        script.sent(),
        json!({"id":"approval-55","result":{"decision":"accept"}})
    );
    assert!(
        matches!(script.event(|e|matches!(e,AgentStreamEvent::InteractionResolved {..})),
        AgentStreamEvent::InteractionResolved {id} if id==approval.id)
    );
    script
        .commands
        .send(LiveCodexCommand::Approval {
            request_id: approval.id,
            option_id: Some("accept".into()),
        })
        .unwrap();
    script.event(|e| matches!(e, AgentStreamEvent::Error(_)));
    assert!(script.outgoing.try_recv().is_err());
    script.server(json!(91),"item/tool/requestUserInput",json!({"isBlocking":true,"questions":[
        {"id":"choice-one","header":"Scope","question":"Choose a scope","options":[{"label":"Small","description":"Keep the current scope"}]},
        {"id":"choice-two","header":"Tests","question":"Choose the checks","isOther":true,"options":null}]}));
    let AgentStreamEvent::Question(first) =
        script.event(|e| matches!(e, AgentStreamEvent::Question(_)))
    else {
        unreachable!()
    };
    assert_eq!(first.prompt, "Choose a scope");
    assert!(first.response_supported);
    assert!(script.events.try_recv().is_err());
    script
        .commands
        .send(LiveCodexCommand::Answer {
            question_id: first.id.clone(),
            answer: "Small".into(),
        })
        .unwrap();
    let AgentStreamEvent::Question(second) =
        script.event(|e| matches!(e, AgentStreamEvent::Question(_)))
    else {
        unreachable!()
    };
    assert_eq!(second.prompt, "Choose the checks");
    assert_ne!(first.id, second.id);
    assert!(script.outgoing.try_recv().is_err());
    script
        .commands
        .send(LiveCodexCommand::Answer {
            question_id: second.id,
            answer: "Run the relevant suite".into(),
        })
        .unwrap();
    assert_eq!(
        script.sent(),
        json!({"id":91,"result":{"answers":{
        "choice-one":{"answers":["Small"]},"choice-two":{"answers":["Run the relevant suite"]}}}})
    );
    assert!(
        matches!(script.event(|event|matches!(event,AgentStreamEvent::UserInputAccepted {..})),
        AgentStreamEvent::UserInputAccepted {text} if text=="Small")
    );
    assert!(
        matches!(script.event(|event|matches!(event,AgentStreamEvent::UserInputAccepted {..})),
        AgentStreamEvent::UserInputAccepted {text} if text=="Run the relevant suite")
    );
    script.complete("completed");
    assert_eq!(
        script.join.join().unwrap().unwrap(),
        LiveCodexOutcome::Completed
    );
}

// SDTEST-2361 → SDUC-554
#[test]
fn sdtest_2361_codex_read_only_and_unsupported_permission_requests_never_grant_authority() {
    let script = Script::new(AgentAccessMode::ReadOnly, None);
    script.start();
    for (id, method, extra) in [
        (
            1,
            "item/commandExecution/requestApproval",
            json!({"command":"write outside workspace"}),
        ),
        (
            2,
            "item/fileChange/requestApproval",
            json!({"reason":"Edit a file"}),
        ),
        (
            3,
            "item/permissions/requestApproval",
            json!({"permissions":{"network":{"enabled":true},"fileSystem":{"write":["/private"]}}}),
        ),
        (
            4,
            "item/tool/requestUserInput",
            json!({"questions":[{"id":"secret","header":"Token","question":"Token?","isSecret":true}]}),
        ),
    ] {
        script.server(json!(id), method, extra);
        let response = script.sent();
        assert_eq!(response["id"], id);
        if id <= 2 {
            assert_eq!(response["result"], json!({"decision":"decline"}));
        } else if id == 3 {
            assert_eq!(response["result"], json!({"permissions":{},"scope":"turn"}));
        } else {
            assert_eq!(response["result"], json!({"answers":{}}));
        }
    }
    script.complete("completed");
    assert_eq!(
        script.join.join().unwrap().unwrap(),
        LiveCodexOutcome::Completed
    );
    assert!(!script.events.try_iter().any(|e| matches!(
        e,
        AgentStreamEvent::Approval(_) | AgentStreamEvent::Question(_)
    )));
}

// SDTEST-2362 → SDUC-554
#[test]
fn sdtest_2362_codex_resume_stream_mapping_redacts_split_secrets_and_deduplicates_final_snapshots()
{
    let script = Script::new(AgentAccessMode::WorkspaceWrite, Some("thread-1"));
    script.start();
    script.notify(
        "turn/plan/updated",
        json!({"threadId":"thread-1","turnId":"turn-1",
        "plan":[{"step":"Inspect","status":"completed"},{"step":"Verify","status":"inProgress"}]}),
    );
    script.notify("thread/tokenUsage/updated",json!({"threadId":"thread-1","turnId":"turn-1",
        "tokenUsage":{"total":{"inputTokens":12,"outputTokens":8,"cachedInputTokens":3,"reasoningOutputTokens":2},
            "last":{"totalTokens":20},"modelContextWindow":100}}));
    for delta in ["Hello sk-secr", "et123 ", "world"] {
        script.notify(
            "item/agentMessage/delta",
            json!({"threadId":"thread-1","turnId":"turn-1","itemId":"message-1","delta":delta}),
        );
    }
    script.notify(
        "item/completed",
        json!({"threadId":"thread-1","turnId":"turn-1",
        "item":{"id":"message-1","type":"agentMessage","text":"Hello sk-secret123 world"}}),
    );
    script.notify("item/completed",json!({"threadId":"thread-1","turnId":"turn-1",
        "item":{"id":"command-1","type":"commandExecution","command":"cargo test API_TOKEN=very-secret",
            "status":"completed","exitCode":0,"aggregatedOutput":"Authorization: Bearer hidden-secret"}}));
    // Child-thread traffic must not bleed into the parent conversation.
    script.notify("item/agentMessage/delta",json!({"threadId":"another-thread","turnId":"turn-1","itemId":"child","delta":"unrelated text "}));
    script.complete("completed");
    assert_eq!(
        script.join.join().unwrap().unwrap(),
        LiveCodexOutcome::Completed
    );
    let events = script.events.try_iter().collect::<Vec<_>>();
    let text = events
        .iter()
        .filter_map(|e| match e {
            AgentStreamEvent::Text(s) | AgentStreamEvent::TextDelta(s) => Some(s.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "Hello [redacted] world");
    assert!(events.iter().any(|e|matches!(e,AgentStreamEvent::Plan(p) if p.steps[1].status==AgentPlanStepStatus::InProgress)));
    assert!(events.iter().any(|e|matches!(e,AgentStreamEvent::Usage(u) if u.input_tokens==Some(12) && u.context_used==Some(20) && u.context_size==Some(100))));
    let formatted = format!("{events:?}");
    assert!(!formatted.contains("very-secret"));
    assert!(!formatted.contains("hidden-secret"));
}

// SDTEST-2363 → SDUC-554
#[test]
fn sdtest_2363_codex_interrupt_and_server_resolution_reject_stale_interactions() {
    let script = Script::new(AgentAccessMode::WorkspaceWrite, None);
    script.start();
    script.server(
        json!(31),
        "item/fileChange/requestApproval",
        json!({"reason":"Apply patch"}),
    );
    let AgentStreamEvent::Approval(approval) =
        script.event(|e| matches!(e, AgentStreamEvent::Approval(_)))
    else {
        unreachable!()
    };
    script.notify(
        "serverRequest/resolved",
        json!({"threadId":"thread-1","requestId":31}),
    );
    script.event(|e| matches!(e, AgentStreamEvent::InteractionResolved { .. }));
    script
        .commands
        .send(LiveCodexCommand::Approval {
            request_id: approval.id,
            option_id: Some("accept".into()),
        })
        .unwrap();
    script.event(|e| matches!(e, AgentStreamEvent::Error(_)));
    assert!(script.outgoing.try_recv().is_err());
    script.commands.send(LiveCodexCommand::Interrupt).unwrap();
    let interrupt = script.sent();
    assert_eq!(interrupt["method"], "turn/interrupt");
    assert_eq!(
        interrupt["params"],
        json!({"threadId":"thread-1","turnId":"turn-1"})
    );
    script.respond(&interrupt, json!({}));
    script.complete("interrupted");
    assert_eq!(
        script.join.join().unwrap().unwrap(),
        LiveCodexOutcome::Interrupted
    );
}

// SDTEST-2364 → SDUC-554
#[test]
fn sdtest_2364_codex_mismatched_sandbox_or_remote_target_fails_before_turn_submission() {
    let mut request = AgentRunRequest::new(
        AgentProvider::Codex,
        AgentTarget::Ssh {
            connection_id: uuid::Uuid::new_v4(),
            connection_label: "Remote".into(),
        },
        AgentAccessMode::ReadOnly,
        "/project",
        None,
        "Inspect",
    );
    assert!(validate(&request, &AgentInputOptions::default()).is_err());
    request.target = AgentTarget::Local;
    let (commands, rx) = mpsc::sync_channel(1);
    let (events, _) = mpsc::channel();
    struct WrongSandbox {
        sent: Vec<Value>,
        next: VecDeque<Value>,
    }
    impl CodexWire for WrongSandbox {
        fn send(&mut self, m: Value) -> Result<()> {
            if m["method"] == "initialize" {
                self.next.push_back(json!({"id":m["id"],"result":{}}));
            }
            if m["method"] == "thread/start" {
                self.next.push_back(json!({"id":m["id"],"result":{
                "thread":{"id":"thread-1"},"cwd":"/project","sandbox":{"type":"dangerFullAccess"}}}));
            }
            self.sent.push(m);
            Ok(())
        }
        fn receive(&mut self, _: Duration) -> Result<Option<Value>> {
            Ok(self.next.pop_front())
        }
    }
    let mut wire = WrongSandbox {
        sent: vec![],
        next: VecDeque::new(),
    };
    assert!(drive(
        &request,
        &AgentInputOptions::default(),
        &mut wire,
        &rx,
        &events,
        &AtomicBool::new(false)
    )
    .unwrap_err()
    .to_string()
    .contains("sandbox"));
    assert!(!wire.sent.iter().any(|r| r["method"] == "turn/start"));
    drop(commands);
}

// SDTEST-2365 → SDUC-554
#[test]
fn sdtest_2365_codex_expired_interaction_is_cancelled_without_reusable_grants() {
    let mut state = RunState::new("thread-1".into(), AgentAccessMode::WorkspaceWrite, 1);
    let (ev, events) = mpsc::channel();
    let (out, outgoing) = mpsc::channel();
    let (_inc, inc) = mpsc::channel();
    let mut wire = FakeWire {
        incoming: inc,
        outgoing: out,
    };
    state.start_turn("turn-1".into(), &ev).unwrap();
    state
        .server_request(
            &mut wire,
            &json!({"id":12,"method":"item/fileChange/requestApproval",
        "params":{"threadId":"thread-1","turnId":"turn-1","itemId":"tool-1"}}),
            &ev,
        )
        .unwrap();
    // Expiration is exercised directly, so this safety test takes no sleep.
    state.force_expiration();
    state.expire(&mut wire, &ev).unwrap();
    assert_eq!(
        outgoing.try_recv().unwrap(),
        json!({"id":12,"result":{"decision":"cancel"}})
    );
    assert!(!state.has_interactions());
    assert!(events
        .try_iter()
        .any(|e| matches!(e, AgentStreamEvent::InteractionResolved { .. })));
}

// SDTEST-2366 → SDUC-554
#[test]
fn sdtest_2366_codex_priority_stop_bypasses_a_full_command_queue() {
    let mut state = RunState::new("thread-1".into(), AgentAccessMode::WorkspaceWrite, 1);
    let (ev, _events) = mpsc::channel();
    let (out, outgoing) = mpsc::channel();
    let (_inc, inc) = mpsc::channel();
    let mut wire = FakeWire {
        incoming: inc,
        outgoing: out,
    };
    let (commands, rx) = mpsc::sync_channel(32);
    for _ in 0..32 {
        commands
            .try_send(LiveCodexCommand::Steer {
                text: "pending".into(),
            })
            .unwrap();
    }
    assert!(commands.try_send(LiveCodexCommand::Interrupt).is_err());
    let cancel = AtomicBool::new(true);
    let mut rpc = Rpc::new();
    assert_eq!(
        run_active(&mut wire, &rx, &ev, &mut rpc, &mut state, &cancel).unwrap(),
        LiveCodexOutcome::Interrupted
    );
    assert!(outgoing.try_recv().is_err());
}

// SDTEST-2367 → SDUC-554
#[test]
fn sdtest_2367_codex_rejected_steering_preserves_drafts_and_priority_stop_finishes() {
    let script = Script::new(AgentAccessMode::WorkspaceWrite, None);
    script.start();
    script
        .commands
        .send(LiveCodexCommand::Steer {
            text: "A new direction".into(),
        })
        .unwrap();
    let steer = script.sent();
    script.incoming.send(json!({"id":steer["id"],"error":{"code":-32600,"message":"Provider denied API_TOKEN=private-value"}})).unwrap();
    assert!(matches!(
        script.event(|event| matches!(event, AgentStreamEvent::UserInputRejected { .. })),
        AgentStreamEvent::UserInputRejected { text } if text == "A new direction"
    ));
    let AgentStreamEvent::Error(error) =
        script.event(|event| matches!(event, AgentStreamEvent::Error(_)))
    else {
        unreachable!()
    };
    assert!(!error.contains("private-value"));
    assert!(!script
        .events
        .try_iter()
        .any(|event| matches!(event, AgentStreamEvent::UserInputAccepted { .. })));
    script.cancel.store(true, Ordering::Release);
    assert_eq!(
        script.join.join().unwrap().unwrap(),
        LiveCodexOutcome::Interrupted
    );
}

// SDTEST-2368 → SDUC-554
#[test]
fn sdtest_2368_codex_file_approval_requires_complete_unredacted_proposed_patch() {
    let script = Script::new(AgentAccessMode::WorkspaceWrite, None);
    script.start();
    script.notify(
        "item/started",
        json!({"threadId":"thread-1","turnId":"turn-1",
        "item":{"id":"tool-1","type":"fileChange","status":"inProgress","changes":[
            {"path":"src/lib.rs","diff":"@@ -1 +1 @@\n-old\n+new\n","kind":{"type":"update"}}]}}),
    );
    script.server(
        json!(81),
        "item/fileChange/requestApproval",
        json!({"reason":"Apply inspected patch"}),
    );
    let AgentStreamEvent::Approval(approval) =
        script.event(|event| matches!(event, AgentStreamEvent::Approval(_)))
    else {
        unreachable!()
    };
    assert!(approval.options.iter().any(|option| option.id == "accept"));
    assert!(approval.title.contains("src/lib.rs"));
    assert!(approval.title.contains("+new"));
    script
        .commands
        .send(LiveCodexCommand::Approval {
            request_id: approval.id,
            option_id: Some("accept".into()),
        })
        .unwrap();
    assert_eq!(
        script.sent(),
        json!({"id":81,"result":{"decision":"accept"}})
    );
    script.server(
        json!(82),
        "item/commandExecution/requestApproval",
        json!({
        "command":"deploy API_TOKEN=private-value","cwd":script.request.workdir}),
    );
    let AgentStreamEvent::Approval(approval) =
        script.event(|event| matches!(event, AgentStreamEvent::Approval(_)))
    else {
        unreachable!()
    };
    assert!(!approval.options.iter().any(|option| option.id == "accept"));
    assert!(!approval.title.contains("private-value"));
    script
        .commands
        .send(LiveCodexCommand::Approval {
            request_id: approval.id,
            option_id: Some("decline".into()),
        })
        .unwrap();
    assert_eq!(
        script.sent(),
        json!({"id":82,"result":{"decision":"decline"}})
    );
    script.complete("completed");
    assert_eq!(
        script.join.join().unwrap().unwrap(),
        LiveCodexOutcome::Completed
    );
}

// SDTEST-2590 — SDUC-557
#[test]
fn browser_mcp_is_injected_into_both_new_and_resumed_codex_threads() {
    for resume in [None, Some("thread-1")] {
        let server = crate::agent_browser_tools::AgentMcpServerLaunch {
            name: "shelldeck_browser".into(),
            command: std::env::temp_dir().join("browser-helper"),
            args: vec!["--mcp".into()],
            env: std::collections::BTreeMap::from([(
                "SHELLDECK_BROWSER_CAPABILITY".into(),
                "fixture-private-token".into(),
            )]),
            claude_config_path: None,
        };
        let script = Script::with_browser(AgentAccessMode::ReadOnly, resume, vec![server]);
        script.start();
        script.complete("completed");
        assert!(matches!(
            script.join.join().unwrap().unwrap(),
            LiveCodexOutcome::Completed
        ));
    }
}
