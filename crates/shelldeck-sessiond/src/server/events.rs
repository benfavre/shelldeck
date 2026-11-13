//! Pane changes pushed to the connections that asked for them
//! (`Subscribe`, docs/design/session-daemon.md section 4).
//!
//! A change is encoded once and queued on every subscribed connection.
//! Queuing never waits: the PTY reader of a pane publishes while it holds
//! the pane lock, so a subscriber that does not read must cost it nothing.
//! Such a subscriber is closed once its queued events pass
//! `ServerConfig::event_queue_cap`, as a slow viewer is dropped.

use super::conn::ConnOut;
use crate::protocol::{DaemonFrame, PaneChange, PaneChangeEvent};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct EventHub {
    subscribers: Mutex<Vec<Arc<ConnOut>>>,
    /// Length of `subscribers`, read without the lock: publishing with no
    /// subscriber, the usual case, is one load.
    count: AtomicUsize,
    cap: usize,
}

impl EventHub {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            count: AtomicUsize::new(0),
            cap,
        }
    }

    /// Whether anyone listens. Publishers use it to skip building an event.
    pub(super) fn is_watched(&self) -> bool {
        self.count.load(Ordering::Relaxed) > 0
    }

    /// Send every later change to `conn`. Asking twice changes nothing.
    pub(super) fn subscribe(&self, conn: &Arc<ConnOut>) {
        let mut subscribers = self.subscribers.lock();
        if !subscribers.iter().any(|known| known.id == conn.id) {
            subscribers.push(conn.clone());
            self.count.store(subscribers.len(), Ordering::Relaxed);
        }
    }

    /// The connection ended.
    pub(super) fn unsubscribe(&self, conn_id: u64) {
        let mut subscribers = self.subscribers.lock();
        subscribers.retain(|conn| conn.id != conn_id);
        self.count.store(subscribers.len(), Ordering::Relaxed);
    }

    /// Queue one change for every subscriber. Never blocks. A subscriber
    /// whose queue is over the cap is closed and forgotten here.
    pub(super) fn publish(&self, pane: Uuid, change: PaneChange) {
        if !self.is_watched() {
            return;
        }
        let Ok(frame) = DaemonFrame::PaneEvent(PaneChangeEvent { pane, change }).encode() else {
            tracing::error!("pane event failed to encode");
            return;
        };
        let mut subscribers = self.subscribers.lock();
        subscribers.retain(|conn| conn.send_event(frame.clone(), self.cap));
        self.count.store(subscribers.len(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{FrameBuffer, PaneChange};

    fn events_of(conn: &ConnOut) -> Vec<PaneChangeEvent> {
        let mut frames = FrameBuffer::new();
        for frame in conn.queued_frames() {
            frames.push(&frame);
        }
        let mut events = Vec::new();
        while let Some(payload) = frames.next_frame().expect("frame") {
            match DaemonFrame::decode(payload).expect("decode") {
                DaemonFrame::PaneEvent(event) => events.push(event),
                other => panic!("{other:?}"),
            }
        }
        events
    }

    // SDTEST-3126 / SDUC-583. A subscriber that reads nothing is closed
    // when its queued events pass the cap, without the publisher ever
    // waiting, and the other subscribers keep getting every event.
    #[test]
    fn sdtest_3126_a_subscriber_that_does_not_read_is_dropped_not_waited_for() {
        let hub = EventHub::new(256);
        let pane = Uuid::from_u128(7);
        // Nobody listens: nothing is built, nothing is kept.
        assert!(!hub.is_watched());
        hub.publish(pane, PaneChange::Removed);

        // No writer thread drains these two: they stand for clients whose
        // socket accepts nothing.
        let slow = Arc::new(ConnOut::new(1));
        let other = Arc::new(ConnOut::new(2));
        hub.subscribe(&slow);
        hub.subscribe(&slow);
        hub.subscribe(&other);
        assert!(hub.is_watched());

        hub.publish(pane, PaneChange::Clients(1));
        assert_eq!(events_of(&slow).len(), 1, "subscribed once");
        assert_eq!(events_of(&other).len(), 1);
        assert!(!slow.is_closed());

        // `other` reads, `slow` does not.
        other.drain();
        let started = std::time::Instant::now();
        let mut sent = 1;
        while !slow.is_closed() {
            hub.publish(pane, PaneChange::Title("t".repeat(100)));
            other.drain();
            sent += 1;
            assert!(sent < 100, "the cap is never reached");
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(!other.is_closed(), "a reader is not dropped with it");
        // What was queued for the closed connection was given back.
        assert!(slow.queued_frames().is_empty());

        // The closed one is forgotten; the other still hears.
        hub.publish(pane, PaneChange::Removed);
        assert_eq!(
            events_of(&other),
            vec![PaneChangeEvent {
                pane,
                change: PaneChange::Removed
            }]
        );
        assert!(slow.queued_frames().is_empty());

        hub.unsubscribe(2);
        assert!(!hub.is_watched());
    }
}
