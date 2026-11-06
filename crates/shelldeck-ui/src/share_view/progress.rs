use super::*;
use std::time::Duration;

#[derive(Clone, Copy, Default)]
pub(super) struct TransferState {
    pub current: usize,
    pub done: u64,
    pub total: u64,
}
#[derive(Default)]
pub(super) struct UploadProgress(parking_lot::Mutex<TransferState>);
impl UploadProgress {
    pub(super) fn begin(&self, current: usize) {
        *self.0.lock() = TransferState {
            current,
            done: 0,
            total: 0,
        };
    }
    pub(super) fn update(&self, done: u64, total: u64) {
        let mut state = self.0.lock();
        state.done = done.min(total);
        state.total = total;
    }
    pub(super) fn read(&self) -> TransferState {
        *self.0.lock()
    }
}
impl ShareView {
    pub(super) fn watch_upload(&mut self, progress: Arc<UploadProgress>, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let active = this
                .update(cx, |this, cx| {
                    let active = this.generation == generation
                        && this.busy
                        && this
                            .upload_progress
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &progress));
                    if active {
                        cx.notify();
                    }
                    active
                })
                .unwrap_or(false);
            if !active {
                break;
            }
        })
        .detach();
    }
    pub(super) fn stop_upload(&mut self, cx: &mut Context<Self>) {
        if self.busy && self.upload_progress.is_some() {
            self.cancelled.store(true, Ordering::Release);
            cx.notify();
        }
    }
}
