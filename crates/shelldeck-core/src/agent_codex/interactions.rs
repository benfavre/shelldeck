//! Exact server-request routing and a sequential host interaction queue.
use super::*;

pub(super) struct RunState {
    pub(super) thread_id: String,
    access: AgentAccessMode,
    turn_id: Option<String>,
    turn_start: u64,
    ready: bool,
    interrupted: Option<Instant>,
    started: Instant,
    interactions: VecDeque<Interaction>,
    file_previews: BTreeMap<String, String>,
    pub(super) text: events::TextEvents,
}
struct Interaction {
    rpc_id: Value,
    deadline: Instant,
    kind: InteractionKind,
}
enum InteractionKind {
    Approval(AgentApprovalRequest),
    Questions {
        questions: VecDeque<Question>,
        answers: BTreeMap<String, Value>,
        accepted: Vec<String>,
    },
}
struct Question {
    original_id: String,
    request: AgentQuestionRequest,
}

impl RunState {
    pub(super) fn new(thread_id: String, access: AgentAccessMode, turn_start: u64) -> Self {
        Self {
            thread_id,
            access,
            turn_id: None,
            turn_start,
            ready: false,
            interrupted: None,
            started: Instant::now(),
            interactions: VecDeque::new(),
            file_previews: BTreeMap::new(),
            text: events::TextEvents::new(),
        }
    }
    fn scope_matches(&self, params: &Value) -> bool {
        params["threadId"].as_str() == Some(self.thread_id.as_str())
            && params["turnId"]
                .as_str()
                .is_some_and(|id| self.turn_id.as_deref() == Some(id))
    }
    pub(super) fn start_turn(
        &mut self,
        id: String,
        events: &Sender<AgentStreamEvent>,
    ) -> Result<()> {
        if self.turn_id.as_ref().is_some_and(|current| current != &id) {
            return Err(failure("Codex reported an unexpected active turn"));
        }
        self.turn_id = Some(id);
        if !self.ready {
            self.ready = true;
            emit(events, AgentStreamEvent::Ready)?;
        }
        Ok(())
    }
    fn show_front(&self, events: &Sender<AgentStreamEvent>) -> Result<()> {
        if let Some(interaction) = self.interactions.front() {
            match &interaction.kind {
                InteractionKind::Approval(request) => {
                    emit(events, AgentStreamEvent::Approval(request.clone()))?
                }
                InteractionKind::Questions { questions, .. } => {
                    if let Some(question) = questions.front() {
                        emit(events, AgentStreamEvent::Question(question.request.clone()))?;
                    }
                }
            }
        }
        Ok(())
    }
    fn resolved(interaction: &Interaction, events: &Sender<AgentStreamEvent>) -> Result<()> {
        let id = match &interaction.kind {
            InteractionKind::Approval(request) => Some(request.id.clone()),
            InteractionKind::Questions { questions, .. } => {
                if let Some(question) = questions.front() {
                    emit(
                        events,
                        AgentStreamEvent::QuestionAnswerRejected {
                            interaction_id: question.request.id.clone(),
                        },
                    )?;
                    Some(question.request.id.clone())
                } else {
                    None
                }
            }
        };
        if let Some(id) = id {
            emit(events, AgentStreamEvent::InteractionResolved { id })?;
        }
        Ok(())
    }
    fn clear(&mut self, events: &Sender<AgentStreamEvent>) -> Result<()> {
        for interaction in self.interactions.drain(..) {
            Self::resolved(&interaction, events)?;
        }
        Ok(())
    }
    fn enqueue(
        &mut self,
        interaction: Interaction,
        events: &Sender<AgentStreamEvent>,
    ) -> Result<()> {
        if self.interactions.len() >= MAX_PENDING_REQUESTS
            || self
                .interactions
                .iter()
                .any(|pending| pending.rpc_id == interaction.rpc_id)
        {
            return Err(failure("Too many or duplicate Codex interaction requests"));
        }
        let was_empty = self.interactions.is_empty();
        self.interactions.push_back(interaction);
        if was_empty {
            self.show_front(events)?;
        }
        Ok(())
    }
    fn respond(
        &mut self,
        command: LiveCodexCommand,
        wire: &mut impl CodexWire,
        rpc: &mut Rpc,
        events: &Sender<AgentStreamEvent>,
    ) -> Result<()> {
        match command {
            LiveCodexCommand::Interrupt => {
                if self.interrupted.is_none() {
                    self.interrupted = Some(Instant::now());
                    self.clear(events)?;
                    if let Some(turn_id) = &self.turn_id {
                        rpc.request(
                            wire,
                            "turn/interrupt",
                            json!({"threadId":self.thread_id,"turnId":turn_id}),
                        )?;
                    }
                }
            }
            LiveCodexCommand::Steer { text } => {
                if self.interrupted.is_some()
                    || text.trim().is_empty()
                    || text.len() > MAX_AGENT_PROMPT_BYTES
                    || rpc.pending.len() >= MAX_PENDING_REQUESTS
                {
                    emit(
                        events,
                        AgentStreamEvent::UserInputRejected { text: safe(&text) },
                    )?;
                    emit(
                        events,
                        AgentStreamEvent::Error(
                            "Cannot steer this Codex turn with that input".into(),
                        ),
                    )?;
                } else if let Some(turn_id) = &self.turn_id {
                    let id = rpc.request(wire, "turn/steer", json!({"threadId":self.thread_id,
                        "expectedTurnId":turn_id,"input":input(&text, &AgentInputOptions::default())}))?;
                    if let Some(pending) = rpc.pending.get_mut(&id) {
                        pending.user_text = Some(safe(&text));
                    }
                } else {
                    emit(
                        events,
                        AgentStreamEvent::UserInputRejected { text: safe(&text) },
                    )?;
                    emit(
                        events,
                        AgentStreamEvent::Error(
                            "Codex is still starting; steering was not sent".into(),
                        ),
                    )?;
                }
            }
            LiveCodexCommand::Approval {
                request_id,
                option_id,
            } => {
                let Some(front) = self.interactions.front() else {
                    return stale(events);
                };
                let InteractionKind::Approval(request) = &front.kind else {
                    return stale(events);
                };
                if request.id != request_id {
                    return stale(events);
                }
                let decision = option_id.as_deref().unwrap_or("cancel");
                if !matches!(decision, "accept" | "decline" | "cancel")
                    || (decision != "cancel" && !request.options.iter().any(|o| o.id == decision))
                    || (decision == "accept" && self.access == AgentAccessMode::ReadOnly)
                {
                    emit(
                        events,
                        AgentStreamEvent::Error(
                            "That Codex approval option is not available".into(),
                        ),
                    )?;
                    return Ok(());
                }
                wire.send(json!({"id":front.rpc_id,"result":{"decision":decision}}))?;
                let resolved = self.interactions.pop_front().unwrap();
                Self::resolved(&resolved, events)?;
                self.show_front(events)?;
            }
            LiveCodexCommand::Answer {
                question_id,
                answer,
            } => {
                let Some(front) = self.interactions.front_mut() else {
                    return reject_question_answer(events, question_id);
                };
                let InteractionKind::Questions {
                    questions,
                    answers,
                    accepted,
                } = &mut front.kind
                else {
                    return reject_question_answer(events, question_id);
                };
                let Some(question) = questions.front() else {
                    return reject_question_answer(events, question_id);
                };
                if question.request.id != question_id {
                    return reject_question_answer(events, question_id);
                }
                if answer.trim().is_empty() || answer.len() > MAX_AGENT_PROMPT_BYTES {
                    emit(
                        events,
                        AgentStreamEvent::QuestionAnswerRejected {
                            interaction_id: question_id,
                        },
                    )?;
                    emit(
                        events,
                        AgentStreamEvent::Error(
                            "Codex answer must contain bounded non-empty text".into(),
                        ),
                    )?;
                    return Ok(());
                }
                let question = questions.pop_front().unwrap();
                let acknowledged_id = question.request.id.clone();
                accepted.push(safe(&answer));
                answers.insert(question.original_id, json!({"answers":[answer]}));
                emit(
                    events,
                    AgentStreamEvent::InteractionResolved {
                        id: question.request.id,
                    },
                )?;
                if questions.is_empty() {
                    wire.send(json!({"id":front.rpc_id,"result":{"answers":answers}}))?;
                    for text in accepted.iter() {
                        emit(
                            events,
                            AgentStreamEvent::UserInputAccepted { text: text.clone() },
                        )?;
                    }
                    self.interactions.pop_front();
                }
                emit(
                    events,
                    AgentStreamEvent::UserInputAcknowledged {
                        interaction_id: acknowledged_id,
                    },
                )?;
                self.show_front(events)?;
            }
        }
        Ok(())
    }

    pub(super) fn server_request(
        &mut self,
        wire: &mut impl CodexWire,
        message: &Value,
        events: &Sender<AgentStreamEvent>,
    ) -> Result<()> {
        let rpc_id = server_id(message)?;
        let params = &message["params"];
        if !self.scope_matches(params) || self.interrupted.is_some() {
            return wire.send(json!({"id":rpc_id,"error":{"code":-32600,"message":"Request is not for the active ShellDeck turn"}}));
        }
        let method = message["method"].as_str().unwrap_or_default();
        let kind = match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                // Read-only may never be upgraded by a callback. grantRoot
                // requests can broaden a whole session: do not offer them.
                if self.access == AgentAccessMode::ReadOnly || !params["grantRoot"].is_null() {
                    wire.send(json!({"id":rpc_id,"result":{"decision":"decline"}}))?;
                    emit(
                        events,
                        AgentStreamEvent::Activity(
                            "Codex permission request declined by the selected access boundary"
                                .into(),
                        ),
                    )?;
                    return Ok(());
                }
                let (mut title, complete) = approval_title(params, method);
                let mut reviewable = complete;
                if method == "item/fileChange/requestApproval" {
                    let preview = params["itemId"]
                        .as_str()
                        .and_then(|id| self.file_previews.get(id));
                    if let Some(preview) = preview {
                        title.push('\n');
                        title.push_str(preview);
                        reviewable &= title.len() <= MAX_AGENT_TRACE_FIELD_BYTES;
                    } else {
                        reviewable = false;
                    }
                }
                let mut options = approval_options(params, method);
                if !reviewable {
                    options.retain(|option| option.kind != AgentApprovalOptionKind::AllowOnce);
                    title = format!(
                        "Complete request details cannot be safely displayed; approval is unavailable.\n{title}"
                    );
                }
                InteractionKind::Approval(
                    AgentApprovalRequest {
                        id: uuid::Uuid::new_v4().to_string(),
                        session_id: self.thread_id.clone(),
                        tool_call_id: valid_id(&params["itemId"])?,
                        title,
                        options,
                    }
                    .bounded(),
                )
            }
            "item/tool/requestUserInput" => {
                let list = params["questions"]
                    .as_array()
                    .filter(|list| !list.is_empty() && list.len() <= 3)
                    .ok_or_else(|| failure("Unsupported Codex question group"))?;
                // Secret prompts need a secure, non-persisted UI. Never collect
                // those through the ordinary conversation composer.
                if list.iter().any(|q| q["isSecret"].as_bool() == Some(true)) {
                    wire.send(json!({"id":rpc_id,"result":{"answers":{}}}))?;
                    emit(
                        events,
                        AgentStreamEvent::Activity(
                            "Codex secret question requires the provider's secure sign-in flow"
                                .into(),
                        ),
                    )?;
                    return Ok(());
                }
                let mut questions = VecDeque::new();
                for q in list {
                    let original_id = valid_id(&q["id"])?;
                    if questions
                        .iter()
                        .any(|existing: &Question| existing.original_id == original_id)
                    {
                        return Err(failure("Duplicate Codex question identifiers"));
                    }
                    let prompt = q["question"]
                        .as_str()
                        .ok_or_else(|| failure("Malformed Codex question"))?;
                    questions.push_back(Question {
                        original_id,
                        request: AgentQuestionRequest {
                            id: uuid::Uuid::new_v4().to_string(),
                            prompt: safe(prompt),
                            options: q["options"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .take(MAX_AGENT_CHOICES)
                                .map(|o| AgentQuestionOption {
                                    label: safe(o["label"].as_str().unwrap_or_default()),
                                    description: safe(
                                        o["description"].as_str().unwrap_or_default(),
                                    ),
                                })
                                .collect(),
                            multiple: false,
                            response_supported: true,
                        }
                        .bounded(),
                    });
                }
                InteractionKind::Questions {
                    questions,
                    answers: BTreeMap::new(),
                    accepted: Vec::new(),
                }
            }
            "item/permissions/requestApproval" => {
                // Do not fabricate a reusable permission grant. A future UI
                // must review the exact network/filesystem subset explicitly.
                wire.send(json!({"id":rpc_id,"result":{"permissions":{},"scope":"turn"}}))?;
                emit(
                    events,
                    AgentStreamEvent::Activity(
                        "Codex requested additional permissions; no permissions were granted"
                            .into(),
                    ),
                )?;
                return Ok(());
            }
            _ => return reject_unsupported(wire, message),
        };
        self.enqueue(
            Interaction {
                rpc_id,
                deadline: Instant::now() + INTERACTION_TIMEOUT,
                kind,
            },
            events,
        )
    }

    #[cfg(test)]
    pub(super) fn force_expiration(&mut self) {
        if let Some(front) = self.interactions.front_mut() {
            front.deadline = Instant::now() - Duration::from_secs(1);
        }
    }
    #[cfg(test)]
    pub(super) fn has_interactions(&self) -> bool {
        !self.interactions.is_empty()
    }

    pub(super) fn expire(
        &mut self,
        wire: &mut impl CodexWire,
        events: &Sender<AgentStreamEvent>,
    ) -> Result<()> {
        let mut changed = false;
        for i in (0..self.interactions.len()).rev() {
            if Instant::now() < self.interactions[i].deadline {
                continue;
            }
            let expired = self.interactions.remove(i).unwrap();
            let result = match &expired.kind {
                InteractionKind::Approval(_) => json!({"decision":"cancel"}),
                InteractionKind::Questions { .. } => json!({"answers":{}}),
            };
            Self::resolved(&expired, events)?;
            wire.send(json!({"id":expired.rpc_id,"result":result}))?;
            changed |= i == 0;
        }
        if changed {
            emit(
                events,
                AgentStreamEvent::Activity(
                    "Codex interaction timed out without granting access".into(),
                ),
            )?;
            self.show_front(events)?;
        }
        Ok(())
    }
}

fn stale(events: &Sender<AgentStreamEvent>) -> Result<()> {
    emit(
        events,
        AgentStreamEvent::Error("That Codex interaction has already ended".into()),
    )
}

fn reject_question_answer(events: &Sender<AgentStreamEvent>, interaction_id: String) -> Result<()> {
    emit(
        events,
        AgentStreamEvent::QuestionAnswerRejected { interaction_id },
    )?;
    stale(events)
}

fn approval_options(params: &Value, method: &str) -> Vec<AgentApprovalOption> {
    let defaults = vec![json!("accept"), json!("decline"), json!("cancel")];
    let decisions = if method == "item/commandExecution/requestApproval" {
        params["availableDecisions"].as_array().unwrap_or(&defaults)
    } else {
        &defaults
    };
    decisions
        .iter()
        .filter_map(|option| match option.as_str()? {
            "accept" => Some(AgentApprovalOption {
                id: "accept".into(),
                label: "Allow this request once".into(),
                kind: AgentApprovalOptionKind::AllowOnce,
            }),
            "decline" => Some(AgentApprovalOption {
                id: "decline".into(),
                label: "Decline this request".into(),
                kind: AgentApprovalOptionKind::RejectOnce,
            }),
            "cancel" => Some(AgentApprovalOption {
                id: "cancel".into(),
                label: "Cancel this request".into(),
                kind: AgentApprovalOptionKind::RejectOnce,
            }),
            _ => None,
        })
        .collect()
}

fn approval_title(params: &Value, method: &str) -> (String, bool) {
    let mut title = if let Some(network) = params
        .get("networkApprovalContext")
        .filter(|v| !v.is_null())
    {
        format!(
            "Network access: {} ({})",
            network["host"].as_str().unwrap_or("unknown host"),
            network["protocol"].as_str().unwrap_or("unknown protocol")
        )
    } else if method == "item/fileChange/requestApproval" {
        "Apply the proposed file changes".into()
    } else {
        format!(
            "Execute: {}",
            params["command"].as_str().unwrap_or("command not supplied")
        )
    };
    for (key, label) in [("cwd", "Directory"), ("reason", "Reason")] {
        if let Some(value) = params[key].as_str() {
            title.push_str(&format!("\n{label}: {value}"));
        }
    }
    if let Some(extra) = params.get("additionalPermissions").filter(|v| !v.is_null()) {
        title.push_str("\nRequested access: ");
        title.push_str(&extra.to_string());
    }
    let redacted = safe(&title);
    let complete = redacted == title
        && title.len() <= MAX_AGENT_TRACE_FIELD_BYTES
        && (method != "item/commandExecution/requestApproval"
            || params["command"].as_str().is_some()
            || !params["networkApprovalContext"].is_null());
    (redacted, complete)
}

pub(super) fn run_active(
    wire: &mut impl CodexWire,
    commands: &Receiver<LiveCodexCommand>,
    events: &Sender<AgentStreamEvent>,
    rpc: &mut Rpc,
    state: &mut RunState,
    cancel: &AtomicBool,
) -> Result<LiveCodexOutcome> {
    loop {
        if cancel.load(Ordering::Acquire) {
            state.clear(events)?;
            return Ok(LiveCodexOutcome::Interrupted);
        }
        // Bound both time and commands per iteration so a chatty host cannot
        // starve stdout (or make cancellation wait behind unlimited steering).
        for _ in 0..32 {
            match commands.try_recv() {
                Ok(command) => state.respond(command, wire, rpc, events)?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    state.clear(events)?;
                    return Ok(LiveCodexOutcome::Interrupted);
                }
            }
        }
        if state
            .interrupted
            .is_some_and(|at| at.elapsed() >= INTERRUPT_TIMEOUT)
        {
            state.clear(events)?;
            return Ok(LiveCodexOutcome::Interrupted);
        }
        if state.started.elapsed() > MAX_RUN_TIME {
            return Err(failure("Codex run reached its time limit"));
        }
        if rpc
            .pending
            .values()
            .any(|pending| pending.at.elapsed() > RPC_TIMEOUT)
        {
            return Err(failure("Codex request timed out; delivery may be unknown"));
        }
        state.expire(wire, events)?;
        let Some(message) = wire.receive(Duration::from_millis(50))? else {
            continue;
        };
        let Some(method) = message["method"].as_str() else {
            let id = message["id"]
                .as_u64()
                .ok_or_else(|| failure("Malformed Codex response identifier"))?;
            let pending = rpc
                .pending
                .remove(&id)
                .ok_or_else(|| failure("Uncorrelated Codex response"))?;
            match rpc_result(&message) {
                Ok(result) if id == state.turn_start => {
                    state.start_turn(valid_id(&result["turn"]["id"])?, events)?
                }
                Ok(_) => {
                    if let Some(text) = pending.user_text {
                        emit(events, AgentStreamEvent::UserInputAccepted { text })?;
                    }
                }
                Err(error) if id == state.turn_start => return Err(error),
                Err(error) => {
                    if let Some(text) = pending.user_text {
                        emit(events, AgentStreamEvent::UserInputRejected { text })?;
                    }
                    emit(events, AgentStreamEvent::Error(error.to_string()))?;
                }
            }
            continue;
        };
        if message.get("id").is_some() {
            state.server_request(wire, &message, events)?;
            continue;
        }
        let params = &message["params"];
        if params["threadId"]
            .as_str()
            .is_some_and(|id| id != state.thread_id)
        {
            continue;
        }
        if method == "turn/started" {
            state.start_turn(valid_id(&params["turn"]["id"])?, events)?;
            continue;
        }
        if method == "serverRequest/resolved" {
            if params["threadId"].as_str() != Some(state.thread_id.as_str()) {
                continue;
            }
            if let Some(i) = state
                .interactions
                .iter()
                .position(|i| i.rpc_id == params["requestId"])
            {
                let interaction = state.interactions.remove(i).unwrap();
                RunState::resolved(&interaction, events)?;
                if i == 0 {
                    state.show_front(events)?;
                }
            }
            continue;
        }
        if method == "turn/completed" {
            if params["threadId"].as_str() != Some(state.thread_id.as_str())
                || params.pointer("/turn/id").and_then(Value::as_str) != state.turn_id.as_deref()
            {
                continue;
            }
            for event in state.text.finish() {
                emit(events, event)?;
            }
            state.clear(events)?;
            return match params["turn"]["status"].as_str() {
                Some("completed") => Ok(LiveCodexOutcome::Completed),
                Some("interrupted") => Ok(LiveCodexOutcome::Interrupted),
                Some("failed") => Err(failure(&safe(
                    params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex turn failed"),
                ))),
                _ => Err(failure("Codex returned an unknown terminal status")),
            };
        }
        if !state.scope_matches(params) {
            continue;
        }
        if method == "item/started" && params["item"]["type"] == "fileChange" {
            if state.file_previews.len() < MAX_PENDING_REQUESTS {
                if let (Ok(id), Some(preview)) = (
                    valid_id(&params["item"]["id"]),
                    events::file_change_approval_preview(&params["item"]),
                ) {
                    state.file_previews.insert(id, preview);
                }
            }
        } else if method == "item/completed" {
            if let Some(id) = params["item"]["id"].as_str() {
                state.file_previews.remove(id);
            }
        }
        for event in state.text.notification(method, params) {
            emit(events, event)?;
        }
    }
}
