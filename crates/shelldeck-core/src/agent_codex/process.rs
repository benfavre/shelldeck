//! Owned child-process I/O. No shell interpolation and no credential logging.
use super::*;
use crate::agent_runtime::{
    configure_local_process, terminate_local_process, AgentStreamFrame, AgentStreamFramer,
    LocalProcessTree, MAX_AGENT_STREAM_LINE_BYTES,
};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;

pub(super) struct ProcessWire {
    child: Child,
    tree: LocalProcessTree,
    // Drop receivers/senders before worker joins to release bounded queues.
    incoming: Option<Receiver<Result<Value>>>,
    outgoing: Option<SyncSender<(Vec<u8>, SyncSender<bool>)>>,
    workers: Vec<JoinHandle<()>>,
}

impl ProcessWire {
    pub(super) fn spawn(program: &Path, workdir: &str) -> Result<Self> {
        let mut command = Command::new(program);
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(workdir.trim())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_local_process(&mut command);
        let mut child = command
            .spawn()
            .map_err(|_| failure("Could not launch local Codex app-server"))?;
        let tree = match LocalProcessTree::capture(child.id()) {
            Ok(tree) => tree,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failure("Could not own the Codex process tree"));
            }
        };
        let (outgoing, output) = mpsc::sync_channel::<(Vec<u8>, SyncSender<bool>)>(2);
        let (incoming_tx, incoming) = mpsc::sync_channel(32);
        let mut owned = Self {
            child,
            tree,
            incoming: Some(incoming),
            outgoing: Some(outgoing),
            workers: Vec::new(),
        };
        let mut stdin = owned
            .child
            .stdin
            .take()
            .ok_or_else(|| failure("Missing Codex input pipe"))?;
        let mut stdout = owned
            .child
            .stdout
            .take()
            .ok_or_else(|| failure("Missing Codex output pipe"))?;
        let mut stderr = owned
            .child
            .stderr
            .take()
            .ok_or_else(|| failure("Missing Codex diagnostic pipe"))?;
        owned.workers.push(
            std::thread::Builder::new()
                .name("codex-input".into())
                .spawn(move || {
                    while let Ok((bytes, ack)) = output.recv() {
                        let ok = stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_ok();
                        let _ = ack.send(ok);
                        if !ok {
                            break;
                        }
                    }
                })
                .map_err(|_| failure("Could not start Codex input worker"))?,
        );
        owned.workers.push(
            std::thread::Builder::new()
                .name("codex-output".into())
                .spawn(move || {
                    let mut framer = AgentStreamFramer::default();
                    let mut buffer = [0; 8192];
                    loop {
                        let count = match stdout.read(&mut buffer) {
                            Ok(count) => count,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                continue;
                            }
                            Err(_) => {
                                let _ = incoming_tx.send(Err(failure("Codex output pipe failed")));
                                return;
                            }
                        };
                        let frames = if count == 0 {
                            framer.finish()
                        } else {
                            framer.push(&buffer[..count])
                        };
                        for frame in frames {
                            let value = match frame {
                                AgentStreamFrame::Oversized => {
                                    Err(failure("Codex protocol frame exceeded its size limit"))
                                }
                                AgentStreamFrame::Line(line) if line.trim().is_empty() => continue,
                                AgentStreamFrame::Line(line) => serde_json::from_str(&line)
                                    .map_err(|_| failure("Malformed Codex protocol frame")),
                            };
                            let failed = value.is_err();
                            if incoming_tx.send(value).is_err() || failed {
                                return;
                            }
                        }
                        if count == 0 {
                            return;
                        }
                    }
                })
                .map_err(|_| failure("Could not start Codex output worker"))?,
        );
        // Diagnostics can contain prompts/credentials; drain without logging,
        // retaining, forwarding to the conversation, or filling a pipe.
        owned.workers.push(
            std::thread::Builder::new()
                .name("codex-diagnostics".into())
                .spawn(move || {
                    let mut buffer = [0; 8192];
                    loop {
                        match stderr.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|_| failure("Could not start Codex diagnostic worker"))?,
        );
        Ok(owned)
    }
}

impl CodexWire for ProcessWire {
    fn send(&mut self, message: Value) -> Result<()> {
        let mut bytes =
            serde_json::to_vec(&message).map_err(|_| failure("Could not encode Codex request"))?;
        if bytes.len() > MAX_AGENT_STREAM_LINE_BYTES {
            return Err(failure("Codex request exceeded its size limit"));
        }
        bytes.push(b'\n');
        let (ack, ack_rx) = mpsc::sync_channel(1);
        self.outgoing
            .as_ref()
            .ok_or_else(|| failure("Codex input closed"))?
            .try_send((bytes, ack))
            .map_err(|_| failure("Codex input is unavailable"))?;
        match ack_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(true) => Ok(()),
            _ => Err(failure("Codex input pipe timed out or closed")),
        }
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<Value>> {
        match self
            .incoming
            .as_ref()
            .ok_or_else(|| failure("Codex output closed"))?
            .recv_timeout(timeout)
        {
            Ok(result) => result.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(failure(
                "Codex app-server disconnected before turn completion",
            )),
        }
    }
}

impl Drop for ProcessWire {
    fn drop(&mut self) {
        let _ = terminate_local_process(&mut self.child, &mut self.tree);
        self.incoming.take();
        self.outgoing.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
