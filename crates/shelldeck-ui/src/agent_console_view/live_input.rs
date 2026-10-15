//! Correlate live submissions with provider acknowledgments before consuming drafts.
use super::*;
use std::collections::hash_map::Entry;

pub(super) struct PendingLiveInput {
    draft: Option<String>,
    acknowledgment: LiveInputAcknowledgment,
}

enum LiveInputAcknowledgment {
    Steering(String),
    Question(String),
}

impl PendingLiveInput {
    pub(super) fn steering(draft: String, submitted: &str) -> Self {
        Self {
            draft: Some(draft),
            acknowledgment: LiveInputAcknowledgment::Steering(
                shelldeck_core::agent_runtime::bounded_agent_context_text(submitted),
            ),
        }
    }

    pub(super) fn question(draft: Option<String>, question_id: String) -> Self {
        Self {
            draft,
            acknowledgment: LiveInputAcknowledgment::Question(question_id),
        }
    }

    fn accepts(&self, event: &AgentStreamEvent) -> bool {
        match (&self.acknowledgment, event) {
            (
                LiveInputAcknowledgment::Steering(expected),
                AgentStreamEvent::UserInputAccepted { text },
            ) => expected == text,
            (
                LiveInputAcknowledgment::Question(expected),
                AgentStreamEvent::UserInputAcknowledged { interaction_id },
            ) => expected == interaction_id,
            _ => false,
        }
    }

    fn consumes_draft(&self, current: &str) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| draft.trim() == current.trim())
    }

    fn rejected_by(&self, event: &AgentStreamEvent) -> bool {
        match (&self.acknowledgment, event) {
            (
                LiveInputAcknowledgment::Steering(expected),
                AgentStreamEvent::UserInputRejected { text },
            ) => expected == text,
            (
                LiveInputAcknowledgment::Question(expected),
                AgentStreamEvent::QuestionAnswerRejected { interaction_id },
            ) => expected == interaction_id,
            _ => false,
        }
    }
}

fn reserve_live_input(
    pending: &mut HashMap<Uuid, PendingLiveInput>,
    run_id: Uuid,
    input: PendingLiveInput,
) -> bool {
    match pending.entry(run_id) {
        Entry::Vacant(entry) => {
            entry.insert(input);
            true
        }
        Entry::Occupied(_) => false,
    }
}

fn accept_live_input(
    pending: &mut HashMap<Uuid, PendingLiveInput>,
    run_id: Uuid,
    event: &AgentStreamEvent,
) -> Option<PendingLiveInput> {
    if pending
        .get(&run_id)
        .is_some_and(|input| input.accepts(event))
    {
        pending.remove(&run_id)
    } else {
        None
    }
}

fn reject_correlated_live_input(
    pending: &mut HashMap<Uuid, PendingLiveInput>,
    run_id: Uuid,
    event: &AgentStreamEvent,
) -> bool {
    if pending
        .get(&run_id)
        .is_some_and(|input| input.rejected_by(event))
    {
        pending.remove(&run_id);
        true
    } else {
        false
    }
}

impl AgentConsoleView {
    pub(super) fn reserve_live_input(
        &mut self,
        run_id: Uuid,
        input: PendingLiveInput,
        cx: &mut Context<Self>,
    ) -> bool {
        if !reserve_live_input(&mut self.pending_live_inputs, run_id, input) {
            return false;
        }
        if let Some(session_id) = self.run_sessions.get(&run_id) {
            self.composer_ui.errors.remove(session_id);
        }
        cx.notify();
        true
    }

    pub(super) fn acknowledge_live_input(
        &mut self,
        run_id: Uuid,
        session_id: Uuid,
        event: &AgentStreamEvent,
        cx: &mut Context<Self>,
    ) {
        // Error/activity events present the failure or cancellation. Negative
        // acknowledgment releases only this input and never consumes its draft.
        if reject_correlated_live_input(&mut self.pending_live_inputs, run_id, event) {
            cx.notify();
            return;
        }
        let Some(input) = accept_live_input(&mut self.pending_live_inputs, run_id, event) else {
            return;
        };
        if let Some(ui) = self.session_ui.get(&session_id) {
            if input.consumes_draft(ui.prompt.read(cx).content()) {
                ui.prompt.update(cx, |state, cx| state.reset(cx));
            }
        }
    }

    /// Enqueue rejection is distinct from provider acceptance. Leave the exact
    /// draft available for retry and release the outstanding-input slot.
    pub fn reject_live_input(&mut self, run_id: Uuid, error: String, cx: &mut Context<Self>) {
        if self.pending_live_inputs.remove(&run_id).is_some() {
            if let Some(session_id) = self.run_sessions.get(&run_id) {
                self.composer_ui.errors.insert(*session_id, error);
            }
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    // SDTEST-2486 — SDUC-554: an unrelated acceptance cannot consume a pending draft.
    #[test]
    fn sdtest_2486_live_acknowledgment_is_scoped_to_run_and_submitted_text() {
        let run = Uuid::new_v4();
        let mut pending = HashMap::new();
        reserve_live_input(
            &mut pending,
            run,
            PendingLiveInput::steering("A".into(), "A"),
        );
        assert!(accept_live_input(
            &mut pending,
            Uuid::new_v4(),
            &AgentStreamEvent::UserInputAccepted { text: "A".into() }
        )
        .is_none());
        assert!(accept_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::UserInputAccepted { text: "B".into() }
        )
        .is_none());
        assert!(accept_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::UserInputAccepted { text: "A".into() }
        )
        .unwrap()
        .consumes_draft("A"));
        assert!(pending.is_empty());
    }

    // SDTEST-2487 — SDUC-554: rapid resubmission never replaces the in-flight input.
    #[test]
    fn sdtest_2487_live_input_reservation_preserves_first_submission() {
        let run = Uuid::new_v4();
        let mut pending = HashMap::new();
        assert!(reserve_live_input(
            &mut pending,
            run,
            PendingLiveInput::steering("A".into(), "A")
        ));
        assert!(!reserve_live_input(
            &mut pending,
            run,
            PendingLiveInput::steering("B".into(), "B")
        ));
        let acknowledged = accept_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::UserInputAccepted { text: "A".into() },
        )
        .unwrap();
        assert!(!acknowledged.consumes_draft("B"));
        assert!(reserve_live_input(
            &mut pending,
            run,
            PendingLiveInput::steering("B".into(), "B")
        ));
        assert!(!reject_correlated_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::Error("Unrelated tool failure".into()),
        ));
        assert!(!reject_correlated_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::UserInputRejected { text: "A".into() },
        ));
        assert!(reject_correlated_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::UserInputRejected { text: "B".into() },
        ));
        // Rejection frees the outstanding slot without accepting/consuming B.
        assert!(reserve_live_input(
            &mut pending,
            run,
            PendingLiveInput::steering("B".into(), "B")
        ));
    }

    // SDTEST-2488 — SDUC-554: accepted expanded/redacted context preserves a newer draft.
    #[test]
    fn sdtest_2488_live_acceptance_matches_delivered_context_without_clearing_new_edits() {
        let submitted = "Inspect\n\nAttached file content";
        let input = PendingLiveInput::steering("Inspect".into(), submitted);
        assert!(input.accepts(&AgentStreamEvent::UserInputAccepted {
            text: shelldeck_core::agent_runtime::bounded_agent_context_text(submitted)
        }));
        assert!(!input.accepts(&AgentStreamEvent::UserInputAccepted {
            text: "Inspect".into()
        }));
        assert!(!input.consumes_draft("Inspect and verify"));
        assert!(input.consumes_draft(" Inspect "));
    }

    // SDTEST-2489 — SDUC-554: question cancellation/expiry is never an answer acknowledgment.
    #[test]
    fn sdtest_2489_question_acknowledgment_requires_exact_accepted_interaction() {
        let input = PendingLiveInput::question(Some("answer".into()), "question-a".into());
        assert!(!input.accepts(&AgentStreamEvent::InteractionResolved {
            id: "question-a".into()
        }));
        assert!(!input.accepts(&AgentStreamEvent::UserInputAcknowledged {
            interaction_id: "question-b".into()
        }));
        assert!(input.accepts(&AgentStreamEvent::UserInputAcknowledged {
            interaction_id: "question-a".into()
        }));
        let option = PendingLiveInput::question(None, "question-a".into());
        assert!(!option.consumes_draft("unsubmitted user text"));
        let run = Uuid::new_v4();
        let mut pending = HashMap::new();
        reserve_live_input(&mut pending, run, input);
        assert!(!reject_correlated_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::QuestionAnswerRejected {
                interaction_id: "question-b".into(),
            }
        ));
        assert!(!reject_correlated_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::InteractionResolved {
                id: "question-a".into(),
            }
        ));
        assert!(reject_correlated_live_input(
            &mut pending,
            run,
            &AgentStreamEvent::QuestionAnswerRejected {
                interaction_id: "question-a".into(),
            }
        ));
        // An expired/cancelled/stale answer can be retried without consuming
        // its draft or mistaking that negative result for acceptance.
        assert!(reserve_live_input(
            &mut pending,
            run,
            PendingLiveInput::question(Some("answer".into()), "question-b".into())
        ));
    }
}
