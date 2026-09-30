use std::collections::HashMap;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Own pending setup cancellation and distinguish retries for the same profile.
#[derive(Default)]
pub(super) struct TunnelStarts(HashMap<Uuid, (Uuid, mpsc::Sender<()>)>);

impl TunnelStarts {
    pub fn contains(&self, forward: Uuid) -> bool {
        self.0.contains_key(&forward)
    }

    pub fn begin(&mut self, forward: Uuid) -> (Uuid, mpsc::Sender<()>, mpsc::Receiver<()>) {
        self.cancel(forward);
        let attempt = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(1);
        self.0.insert(forward, (attempt, tx.clone()));
        (attempt, tx, rx)
    }

    /// A stopped or replaced attempt cannot publish success, failure or timeout.
    pub fn finish(&mut self, forward: Uuid, attempt: Uuid) -> bool {
        if self.0.get(&forward).is_some_and(|(id, _)| *id == attempt) {
            self.0.remove(&forward);
            true
        } else {
            false
        }
    }

    pub fn cancel(&mut self, forward: Uuid) -> bool {
        if let Some((_, tx)) = self.0.remove(&forward) {
            let _ = tx.try_send(());
            true
        } else {
            false
        }
    }

    pub fn cancel_all(&mut self) {
        for (_, (_, tx)) in self.0.drain() {
            let _ = tx.try_send(());
        }
    }
}

impl Drop for TunnelStarts {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-1943
    #[tokio::test]
    async fn stop_retry_and_account_shutdown_retire_pending_tunnel_results() {
        let mut starts = TunnelStarts::default();
        let forward = Uuid::new_v4();
        let (old, _old_tx, mut old_rx) = starts.begin(forward);
        assert!(starts.cancel(forward));
        assert_eq!(old_rx.recv().await, Some(()));
        let (new, _new_tx, mut new_rx) = starts.begin(forward);
        // Delayed success/error for the old setup must not retire the retry.
        assert!(!starts.finish(forward, old));
        assert!(starts.contains(forward));
        assert!(matches!(
            new_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(starts.finish(forward, new));
        assert!(!starts.finish(forward, new));

        let (pending, _tx, mut rx) = starts.begin(forward);
        let other = Uuid::new_v4();
        let (second, _tx2, mut rx2) = starts.begin(other);
        starts.cancel_all();
        assert_eq!(rx.recv().await, Some(()));
        assert_eq!(rx2.recv().await, Some(()));
        assert!(!starts.finish(forward, pending));
        assert!(!starts.finish(other, second));
        let (retry, _tx3, mut rx3) = starts.begin(forward);
        assert!(!starts.finish(forward, pending));
        drop(starts);
        assert_eq!(rx3.recv().await, Some(()));
        assert_ne!(retry, pending);
    }
}
