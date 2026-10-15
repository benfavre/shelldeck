use super::*;

#[derive(Default)]
struct Queue {
    events: VecDeque<Event>,
    frames: HashMap<(BrowserId, PaintKind), (Event, Vec<u8>)>,
    done: bool,
}
#[derive(Clone)]
pub(super) struct Output(Arc<(Mutex<Queue>, Condvar)>);
impl Output {
    pub fn new() -> Self {
        Self(Arc::new((Mutex::new(Queue::default()), Condvar::new())))
    }
    pub fn send(&self, event: Event) {
        let (lock, signal) = &*self.0;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        if queue.done {
            return;
        }
        if let Some(key) = coalescing_key(&event) {
            if let Some(index) = queue
                .events
                .iter()
                .position(|previous| coalescing_key(previous) == Some(key))
            {
                queue.events.remove(index);
                queue.events.push_back(event);
                signal.notify_one();
                return;
            }
        }
        if queue.events.len() >= 256 {
            // A blocked consumer must not grow an unbounded metadata/response
            // backlog on Chromium's UI thread. Fail the pipe explicitly.
            queue.events.clear();
            queue.frames.clear();
            queue.done = true;
            queue.events.push_back(Event::Error {
                browser_id: None,
                message: "The browser event consumer stopped keeping up; the helper was closed"
                    .into(),
            });
            signal.notify_one();
            drop(queue);
            post(Work::Command(Command::Shutdown));
            return;
        }
        queue.events.push_back(event);
        signal.notify_one();
    }
    pub fn paint(&self, event: Event, pixels: Vec<u8>) {
        let Event::Paint {
            browser_id, kind, ..
        } = event
        else {
            return;
        };
        let (lock, signal) = &*self.0;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        if !queue.done {
            queue.frames.insert((browser_id, kind), (event, pixels));
            signal.notify_one();
        }
    }
    pub fn forget_frames(&self, id: BrowserId) {
        self.0
             .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .frames
            .retain(|(browser_id, _), _| *browser_id != id);
    }
    pub fn finish(&self) {
        let (lock, signal) = &*self.0;
        lock.lock().unwrap_or_else(|e| e.into_inner()).done = true;
        signal.notify_one();
    }
    pub fn write(&self) -> io::Result<()> {
        let mut stdout = BufWriter::new(io::stdout().lock());
        loop {
            let (event, pixels) = {
                let (lock, signal) = &*self.0;
                let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(event) = queue.events.pop_front() {
                        break (event, Vec::new());
                    }
                    if let Some(key) = queue.frames.keys().next().copied() {
                        break queue.frames.remove(&key).unwrap();
                    }
                    if queue.done {
                        return Ok(());
                    }
                    queue = signal.wait(queue).unwrap_or_else(|e| e.into_inner());
                }
            };
            protocol::write_packet(&mut stdout, &event, &pixels)?;
        }
    }
}

fn coalescing_key(event: &Event) -> Option<(u8, BrowserId, u64)> {
    match event {
        Event::State { state } => Some((0, state.browser_id, 0)),
        Event::CursorChanged { browser_id, .. } => Some((1, *browser_id, 0)),
        Event::TextSelectionChanged { browser_id, .. } => Some((2, *browser_id, 0)),
        Event::ImeRangeChanged { browser_id, .. } => Some((3, *browser_id, 0)),
        Event::Popup { browser_id, .. } => Some((4, *browser_id, 0)),
        Event::DownloadProgress {
            browser_id,
            download_id,
            ..
        } => Some((5, *browser_id, *download_id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-2554 — SDUC-557: a page changing titles/cursors cannot queue thousands of stale UI events.
    #[test]
    fn sdtest_2554_browser_page_metadata_coalesces() {
        let output = Output::new();
        for index in 0..10000 {
            output.send(Event::State {
                state: BrowserState {
                    browser_id: 1,
                    url: "https://example.test".into(),
                    title: index.to_string(),
                    loading: false,
                    can_go_back: false,
                    can_go_forward: false,
                },
            });
        }
        let queue = output.0 .0.lock().unwrap();
        assert_eq!(queue.events.len(), 1);
        assert!(matches!(queue.events.front(),Some(Event::State { state }) if state.title=="9999"));
        assert_eq!(clipped("é🦀".into(), 4), "é");
    }
    // SDTEST-2550 — SDUC-557: a stalled host retains only the latest view/popup frame per tab.
    #[test]
    fn sdtest_2550_browser_backpressure_coalesces_frames() {
        let output = Output::new();
        for value in 0..20 {
            output.paint(
                Event::Paint {
                    browser_id: 1,
                    kind: PaintKind::View,
                    width: 1,
                    height: 1,
                    dirty_rects: vec![],
                },
                vec![value; 4],
            );
        }
        output.paint(
            Event::Paint {
                browser_id: 1,
                kind: PaintKind::Popup,
                width: 1,
                height: 1,
                dirty_rects: vec![],
            },
            vec![21; 4],
        );
        let queue = output.0 .0.lock().unwrap();
        assert_eq!(queue.frames.len(), 2);
        assert_eq!(queue.frames.get(&(1, PaintKind::View)).unwrap().1, [19; 4]);
        drop(queue);
        output.forget_frames(1);
        assert!(output.0 .0.lock().unwrap().frames.is_empty());
    }
}
