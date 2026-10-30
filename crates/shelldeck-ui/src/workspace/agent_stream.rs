//! Fair, ordered delivery from provider workers to the UI. Completion never
//! overtakes queued output; one busy provider cannot drain forever in a frame.
use shelldeck_core::agent_runtime::AgentStreamEvent;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

pub(super) const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const MAX_EVENTS_PER_FRAME: usize = 64;
type Done = (Option<i32>, Option<String>);

pub(super) struct AgentEventPump {
    events: Receiver<AgentStreamEvent>,
    done: Receiver<Done>,
    completion: Option<Done>,
    finished: bool,
    started: Instant,
    first_text_seen: bool,
}

impl AgentEventPump {
    pub fn new(events: Receiver<AgentStreamEvent>, done: Receiver<Done>) -> Self {
        Self {
            events,
            done,
            completion: None,
            finished: false,
            started: Instant::now(),
            first_text_seen: false,
        }
    }

    pub fn poll(&mut self) -> (Vec<AgentStreamEvent>, Option<Done>) {
        if self.finished {
            return (Vec::new(), None);
        }
        if self.completion.is_none() {
            self.completion = match self.done.try_recv() {
                Ok(done) => Some(done),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some((
                    None,
                    Some(crate::t!("agents.error.runtime_lost").to_string()),
                )),
            };
        }
        let events = self
            .events
            .try_iter()
            .take(MAX_EVENTS_PER_FRAME)
            .collect::<Vec<_>>();
        if !self.first_text_seen && events.iter().any(|event| {
            matches!(event, AgentStreamEvent::Text(text) | AgentStreamEvent::TextDelta(text) | AgentStreamEvent::TextSnapshot(text) if !text.is_empty())
        }) {
            self.first_text_seen = true;
            tracing::debug!(elapsed_ms = self.started.elapsed().as_millis() as u64,
                "agent stream bridge delivered first text");
        }
        // Done is sent only after producers finish. A full batch may still have
        // trailing output; leave the completion pending until the next frame.
        let completion = if events.len() < MAX_EVENTS_PER_FRAME {
            self.completion.take()
        } else {
            None
        };
        self.finished = completion.is_some();
        if self.finished {
            tracing::debug!(
                elapsed_ms = self.started.elapsed().as_millis() as u64,
                "agent stream bridge drained completion"
            );
        }
        (events, completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    // SDTEST-2793 — flood output remains ordered and completion cannot overtake it.
    #[test]
    fn bounded_frames_preserve_trailing_output_and_completion() {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let mut pump = AgentEventPump::new(rx, done_rx);
        assert!(pump.poll().0.is_empty());
        for i in 0..1000 {
            tx.send(AgentStreamEvent::TextDelta(i.to_string())).unwrap();
        }
        done_tx.send((Some(0), None)).unwrap();
        drop(tx);
        drop(done_tx);
        let mut received = Vec::new();
        let mut frames = 0;
        loop {
            let (events, done) = pump.poll();
            frames += 1;
            assert!(events.len() <= MAX_EVENTS_PER_FRAME);
            for event in events {
                let AgentStreamEvent::TextDelta(text) = event else {
                    panic!("unexpected event")
                };
                received.push(text.parse::<usize>().unwrap());
            }
            if let Some(done) = done {
                assert_eq!(done, (Some(0), None));
                break;
            }
            assert!(frames < 20);
        }
        assert_eq!(received, (0..1000).collect::<Vec<_>>());
        assert_eq!(frames, 16);
        assert_eq!(pump.poll().1, None);
    }

    #[test]
    fn worker_loss_drains_output_and_clean_stop_stays_cancelled() {
        for done in [Some((None, None)), None] {
            let (tx, rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            tx.send(AgentStreamEvent::TextDelta("last words".into()))
                .unwrap();
            if let Some(done) = done.clone() {
                done_tx.send(done).unwrap();
            }
            drop(tx);
            drop(done_tx);
            let mut pump = AgentEventPump::new(rx, done_rx);
            let (events, result) = pump.poll();
            assert_eq!(events.len(), 1);
            let (exit, error) = result.unwrap();
            assert!(exit.is_none());
            assert_eq!(error.is_some(), done.is_none());
        }
    }
}
