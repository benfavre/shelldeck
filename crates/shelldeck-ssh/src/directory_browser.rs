//! Read-only directory navigation on one saved SSH connection.
//!
//! The browser owns a dedicated runtime/session, not a terminal or the shared
//! connection pool. Closing it cancels admission and outstanding work without
//! blocking the UI thread. Paths remain JSON stdin to the fixed agent-files helper.
use crate::{agent_files, client::SshClient, session::SshSession};
use async_trait::async_trait;
use shelldeck_core::models::Connection;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const QUEUE_CAPACITY: usize = 4;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const DISCONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_PATH_BYTES: usize = 4096;
type Listing = Result<agent_files::Directory, String>;

struct Request {
    path: String,
    reply: oneshot::Sender<Listing>,
}

/// A disposable folder-picker connection. No provider or terminal is launched.
pub struct DirectoryBrowser {
    requests: mpsc::Sender<Request>,
    cancel: Option<oneshot::Sender<()>>,
}

impl DirectoryBrowser {
    pub fn start(connection: Connection) -> Result<Self, String> {
        Self::start_with(SavedConnection(connection)).map(|(browser, _finished)| browser)
    }

    /// Queue one listing without waiting on the caller's thread. `None` starts
    /// at remote HOME; explicit paths are absolute remote POSIX paths.
    pub fn list(&self, path: Option<String>) -> oneshot::Receiver<Listing> {
        let (reply, receiver) = oneshot::channel();
        let path = match path {
            None => "~".to_owned(),
            Some(path)
                if path.starts_with('/')
                    && !path.contains('\0')
                    && path.len() <= MAX_PATH_BYTES =>
            {
                path
            }
            Some(_) => {
                let _ = reply.send(Err(
                    "Choose an absolute remote path of at most 4096 UTF-8 bytes without NUL".into(),
                ));
                return receiver;
            }
        };
        if let Err(error) = self.requests.try_send(Request { path, reply }) {
            let message = match &error {
                mpsc::error::TrySendError::Full(_) => {
                    "Directory browser is busy; retry after the current listing"
                }
                mpsc::error::TrySendError::Closed(_) => {
                    "Directory browser is closed; reconnect to retry"
                }
            };
            let _ = error.into_inner().reply.send(Err(message.into()));
        }
        receiver
    }

    fn start_with<B: BrowserBackend>(backend: B) -> Result<(Self, oneshot::Receiver<()>), String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let (requests, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (cancel, cancelled) = oneshot::channel();
        let (finished, completion) = oneshot::channel();
        std::thread::Builder::new()
            .name("ssh-directory-browser".into())
            .spawn(move || {
                runtime.block_on(run(backend, receiver, cancelled));
                // Drop all runtime tasks/sockets before reporting worker completion.
                drop(runtime);
                let _ = finished.send(());
            })
            .map_err(|error| error.to_string())?;
        Ok((
            Self {
                requests,
                cancel: Some(cancel),
            },
            completion,
        ))
    }
}

impl Drop for DirectoryBrowser {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

#[async_trait]
trait BrowserBackend: Send + 'static {
    type Session: BrowserSession;
    async fn connect(self) -> Result<Self::Session, String>;
}

#[async_trait]
trait BrowserSession: Send + Sync {
    async fn list(&self, path: &str) -> Listing;
    async fn disconnect(&self);
}

struct SavedConnection(Connection);

#[async_trait]
impl BrowserBackend for SavedConnection {
    type Session = SshSession;
    async fn connect(self) -> Result<SshSession, String> {
        SshClient::new()
            .connect(&self.0)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl BrowserSession for SshSession {
    async fn list(&self, path: &str) -> Listing {
        // The fixed helper expands '~' in the path; '/' permits browsing any
        // directory readable by this authenticated SSH account, not writing it.
        agent_files::list(self, "/", path)
            .await
            .map_err(|error| error.to_string())
    }
    async fn disconnect(&self) {
        let _ = SshSession::disconnect(self).await;
    }
}

async fn run<B: BrowserBackend>(
    backend: B,
    mut requests: mpsc::Receiver<Request>,
    mut cancelled: oneshot::Receiver<()>,
) {
    let connection = tokio::select! {
        biased;
        _ = &mut cancelled => return,
        result = tokio::time::timeout(CONNECT_TIMEOUT, backend.connect()) => {
            result.unwrap_or_else(|_| Err("SSH directory connection timed out".into()))
        }
    };
    loop {
        let request = tokio::select! {
            biased;
            _ = &mut cancelled => break,
            request = requests.recv() => match request { Some(request) => request, None => break },
        };
        let Request { path, mut reply } = request;
        if reply.is_closed() {
            continue;
        }
        let result = match &connection {
            Err(error) => Err(error.clone()),
            Ok(session) => tokio::select! {
                biased;
                _ = &mut cancelled => {
                    let _ = reply.send(Err("Directory browser closed".into()));
                    break;
                },
                _ = reply.closed() => continue,
                result = tokio::time::timeout(LIST_TIMEOUT, session.list(&path)) => {
                    result.unwrap_or_else(|_| Err("Remote directory listing timed out".into()))
                }
            },
        };
        let _ = reply.send(result);
    }
    requests.close();
    while let Ok(request) = requests.try_recv() {
        let _ = request.reply.send(Err("Directory browser closed".into()));
    }
    if let Ok(session) = connection {
        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, session.disconnect()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    #[derive(Default)]
    struct Evidence {
        paths: Mutex<Vec<String>>,
        cancelled: AtomicUsize,
        disconnected: AtomicUsize,
        dropped: AtomicUsize,
    }
    struct Fake {
        evidence: Arc<Evidence>,
        entered: std::sync::mpsc::Sender<()>,
        hang_connect: bool,
        hang_list: bool,
    }
    struct FakeSession(Fake);
    struct CancelGuard(Arc<Evidence>);
    impl Drop for CancelGuard {
        fn drop(&mut self) {
            self.0.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Drop for FakeSession {
        fn drop(&mut self) {
            self.0.evidence.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[async_trait]
    impl BrowserBackend for Fake {
        type Session = FakeSession;
        async fn connect(self) -> Result<FakeSession, String> {
            if self.hang_connect {
                let _guard = CancelGuard(self.evidence.clone());
                self.entered.send(()).unwrap();
                std::future::pending::<()>().await;
            }
            Ok(FakeSession(self))
        }
    }
    #[async_trait]
    impl BrowserSession for FakeSession {
        async fn list(&self, path: &str) -> Listing {
            self.0.evidence.paths.lock().unwrap().push(path.to_owned());
            if self.0.hang_list {
                let _guard = CancelGuard(self.0.evidence.clone());
                self.0.entered.send(()).unwrap();
                std::future::pending::<()>().await;
            }
            Ok(agent_files::Directory {
                root: "/".into(),
                path: if path == "~" {
                    "/home/fixture".into()
                } else {
                    path.into()
                },
                entries: vec![],
                truncated: false,
            })
        }
        async fn disconnect(&self) {
            self.0.evidence.disconnected.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn fixture(
        hang_connect: bool,
        hang_list: bool,
    ) -> (
        DirectoryBrowser,
        oneshot::Receiver<()>,
        Arc<Evidence>,
        std::sync::mpsc::Receiver<()>,
    ) {
        let evidence = Arc::new(Evidence::default());
        let (entered, receiver) = std::sync::mpsc::channel();
        let (browser, done) = DirectoryBrowser::start_with(Fake {
            evidence: evidence.clone(),
            entered,
            hang_connect,
            hang_list,
        })
        .unwrap();
        (browser, done, evidence, receiver)
    }
    fn finished(mut done: oneshot::Receiver<()>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match done.try_recv() {
                Ok(()) => return,
                Err(oneshot::error::TryRecvError::Empty)
                    if std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2))
                }
                other => panic!("worker failed to finish: {other:?}"),
            }
        }
    }

    #[test]
    fn sdtest_2714_directory_navigation_preserves_exact_paths_and_one_session() {
        let (browser, done, evidence, _) = fixture(false, false);
        assert_eq!(
            browser.list(None).blocking_recv().unwrap().unwrap().path,
            "/home/fixture"
        );
        let path = "/tmp/a 'quoted' $(not-a-command)\nfolder";
        assert_eq!(
            browser
                .list(Some(path.into()))
                .blocking_recv()
                .unwrap()
                .unwrap()
                .path,
            path
        );
        for invalid in ["relative", "~", "/nul\0path"] {
            assert!(browser
                .list(Some(invalid.into()))
                .blocking_recv()
                .unwrap()
                .is_err());
        }
        assert_eq!(*evidence.paths.lock().unwrap(), vec!["~", path]);
        drop(browser);
        finished(done);
        assert_eq!(evidence.disconnected.load(Ordering::SeqCst), 1);
        assert_eq!(evidence.dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn sdtest_2715_close_cancels_connection_and_bounds_pending_admission() {
        let (browser, done, evidence, entered) = fixture(true, false);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        let pending = (0..QUEUE_CAPACITY)
            .map(|_| browser.list(None))
            .collect::<Vec<_>>();
        assert!(browser
            .list(None)
            .blocking_recv()
            .unwrap()
            .unwrap_err()
            .contains("busy"));
        drop(browser);
        finished(done);
        assert_eq!(evidence.cancelled.load(Ordering::SeqCst), 1);
        for request in pending {
            assert!(request.blocking_recv().is_err());
        }
    }

    #[test]
    fn sdtest_2716_close_cancels_listing_disconnects_and_finishes_worker() {
        let (browser, done, evidence, entered) = fixture(false, true);
        let pending = browser.list(None);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(browser);
        finished(done);
        assert!(pending.blocking_recv().unwrap().is_err());
        assert_eq!(evidence.cancelled.load(Ordering::SeqCst), 1);
        assert_eq!(evidence.disconnected.load(Ordering::SeqCst), 1);
        assert_eq!(evidence.dropped.load(Ordering::SeqCst), 1);
    }
}
