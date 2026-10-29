//! Standard SSH existing-folder preparation: retain process cwd until explicit resume.
use super::SshChannel;
use crate::{Result, SshError};
use russh::{client, Channel, ChannelMsg};
use std::time::Duration;

const MAX_STARTUP_BYTES: usize = 64 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

// The requested directory and random token are positional data, never script source.
// Save the server's actual PTY modes before suppressing control-input echo.
const STARTUP_SCRIPT: &str = r#"set -eu
case "$1" in /*) ;; *) exit 64 ;; esac
CDPATH= cd -P "$1" || exit 72
shell=${SHELL:-/bin/sh}
case "$shell" in /*) ;; *) exit 69 ;; esac
[ -x "$shell" ] || exit 69
saved=$(stty -g) || exit 70
trap 'stty "$saved" 2>/dev/null || :' 0
stty -echo || exit 70
printf '\036SHELLDECK_FOLDER_%s_PREPARED\037' "$2"
IFS= read -r reply || exit 74
[ "$reply" = "$2" ] || exit 74
stty "$saved" || exit 70
trap - 0
printf '\036SHELLDECK_FOLDER_%s_RESUMED\037' "$2"
exec "$shell" -i
"#;

fn startup_command(path: &str, nonce: &str) -> Result<String> {
    if path.len() > 4096
        || shelldeck_core::config::workspace_catalog::RemotePosixPath::new(path).is_err()
    {
        return Err(error("invalid absolute folder path"));
    }
    Ok(format!(
        "sh -c {} shelldeck-folder {} {}",
        crate::tmux::shell_quote(STARTUP_SCRIPT),
        crate::tmux::shell_quote(path),
        crate::tmux::shell_quote(nonce),
    ))
}

fn marker(nonce: &str, phase: &str) -> Vec<u8> {
    format!("\x1eSHELLDECK_FOLDER_{nonce}_{phase}\x1f").into_bytes()
}

struct StartupHandshake {
    marker: Vec<u8>,
    buffer: Vec<u8>,
}
impl StartupHandshake {
    fn new(nonce: &str, phase: &str) -> Self {
        Self {
            marker: marker(nonce, phase),
            buffer: Vec::new(),
        }
    }

    /// Remove only our exact nonce-bound marker; preserve banners and terminal bytes.
    fn feed(&mut self, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_STARTUP_BYTES {
            return Err(error("startup output exceeded its limit"));
        }
        self.buffer.extend_from_slice(bytes);
        let Some(start) = self
            .buffer
            .windows(self.marker.len())
            .position(|part| part == self.marker)
        else {
            return Ok(None);
        };
        self.buffer.drain(start..start + self.marker.len());
        Ok(Some(std::mem::take(&mut self.buffer)))
    }
}

/// An existing remote folder held as a process cwd. No interactive shell runs
/// until `resume`; dropping a prepared or in-flight value closes its channel.
pub struct PreparedFolderShell {
    channel: Option<Channel<client::Msg>>,
    nonce: String,
    initial: Vec<u8>,
    runtime: tokio::runtime::Handle,
}

impl PreparedFolderShell {
    pub(super) async fn prepare(
        channel: Channel<client::Msg>,
        path: &str,
        rows: u32,
        cols: u32,
    ) -> Result<Self> {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let mut prepared = Self {
            channel: Some(channel),
            nonce,
            initial: Vec::new(),
            runtime: tokio::runtime::Handle::current(),
        };
        let command = startup_command(path, &prepared.nonce)?;
        tokio::time::timeout(STARTUP_TIMEOUT, async {
            let channel = prepared.channel.as_ref().unwrap();
            channel
                .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
                .await
                .map_err(|cause| error(&format!("PTY request failed: {cause}")))?;
            channel
                .exec(true, command)
                .await
                .map_err(|cause| error(&cause.to_string()))?;
            prepared.read_marker("PREPARED").await
        })
        .await
        .map_err(|_| SshError::Timeout)??;
        Ok(prepared)
    }

    async fn read_marker(&mut self, phase: &str) -> Result<()> {
        let mut handshake = StartupHandshake::new(&self.nonce, phase);
        let channel = self.channel.as_mut().unwrap();
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    if let Some(bytes) = handshake.feed(&data)? {
                        if self.initial.len().saturating_add(bytes.len()) > MAX_STARTUP_BYTES {
                            return Err(error("startup output exceeded its limit"));
                        }
                        self.initial.extend(bytes);
                        return Ok(());
                    }
                }
                ChannelMsg::Failure => return Err(error("server refused folder shell request")),
                ChannelMsg::Eof
                | ChannelMsg::Close
                | ChannelMsg::ExitStatus { .. }
                | ChannelMsg::ExitSignal { .. } => {
                    return Err(error(
                        "folder shell ended before readiness; check the folder and account shell",
                    ));
                }
                _ => {}
            }
        }
        Err(error("connection closed before readiness"))
    }

    pub async fn resume(mut self) -> Result<SshChannel> {
        tokio::time::timeout(STARTUP_TIMEOUT, async {
            let control = format!("{}\n", self.nonce);
            self.channel
                .as_ref()
                .unwrap()
                .data(std::io::Cursor::new(control.into_bytes()))
                .await
                .map_err(|cause| error(&cause.to_string()))?;
            self.read_marker("RESUMED").await
        })
        .await
        .map_err(|_| SshError::Timeout)??;
        Ok(SshChannel {
            channel: self.channel.take().unwrap(),
            initial: std::mem::take(&mut self.initial),
        })
    }

    pub async fn release(mut self) -> Result<()> {
        if let Some(channel) = self.channel.as_ref() {
            // Retain Drop custody across both awaits. Cancellation or a failed
            // close must still schedule cleanup on the owning runtime.
            close_channel(channel).await?;
            self.channel.take();
        }
        Ok(())
    }
}

async fn close_channel(channel: &Channel<client::Msg>) -> Result<()> {
    bounded_close_attempts(channel.eof(), channel.close(), Duration::from_secs(1)).await
}

async fn bounded_close_attempts<E: std::fmt::Display>(
    eof: impl std::future::Future<Output = std::result::Result<(), E>>,
    close: impl std::future::Future<Output = std::result::Result<(), E>>,
    timeout: Duration,
) -> Result<()> {
    // Separate budgets: a blocked EOF must not consume the Close attempt.
    let eof = tokio::time::timeout(timeout, eof).await;
    let close = tokio::time::timeout(timeout, close).await;
    close
        .map_err(|_| error("channel close timed out"))?
        .map_err(|cause| error(&cause.to_string()))?;
    eof.map_err(|_| error("channel EOF timed out"))?
        .map_err(|cause| error(&cause.to_string()))
}

impl Drop for PreparedFolderShell {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.take() {
            self.runtime.spawn(async move {
                let _ = close_channel(&channel).await;
            });
        }
    }
}

fn error(message: &str) -> SshError {
    SshError::Channel(format!("remote folder: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2740 — cleanup must attempt Close even if EOF stalls or fails.
    #[tokio::test]
    async fn folder_release_attempts_close_after_eof_failure_or_timeout() {
        use std::sync::atomic::{AtomicBool, Ordering};
        for stalled in [false, true] {
            let closed = AtomicBool::new(false);
            let eof = async {
                if stalled {
                    std::future::pending::<()>().await;
                }
                Err::<(), _>("EOF failed")
            };
            let close = async {
                closed.store(true, Ordering::SeqCst);
                Ok::<(), &str>(())
            };
            assert!(bounded_close_attempts(eof, close, Duration::from_millis(5))
                .await
                .is_err());
            assert!(closed.load(Ordering::SeqCst));
        }
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            bounded_close_attempts(
                async { Ok::<(), &str>(()) },
                std::future::pending::<std::result::Result<(), &str>>(),
                Duration::from_millis(5),
            ),
        )
        .await
        .expect("Close itself must also have a deadline");
        assert!(result.unwrap_err().to_string().contains("close timed out"));
    }

    // SDTEST-2729 — readiness is nonce-bound, fragmented, bounded and lossless.
    #[test]
    fn folder_handshake_preserves_output_and_rejects_missing_or_wrong_nonce() {
        let nonce = "0123456789abcdef0123456789abcdef";
        let expected = marker(nonce, "PREPARED");
        for split in 0..=expected.len() {
            let mut parser = StartupHandshake::new(nonce, "PREPARED");
            assert!(parser.feed(b"banner\r\n").unwrap().is_none());
            let first = parser.feed(&expected[..split]).unwrap();
            let output = if split == expected.len() {
                first.unwrap()
            } else {
                assert!(first.is_none());
                parser
                    .feed(&[&expected[split..], b"terminal"].concat())
                    .unwrap()
                    .unwrap()
            };
            assert_eq!(
                output,
                if split == expected.len() {
                    b"banner\r\n".to_vec()
                } else {
                    b"banner\r\nterminal".to_vec()
                }
            );
        }
        let mut parser = StartupHandshake::new(nonce, "PREPARED");
        assert!(parser.feed(&marker("other", "PREPARED")).unwrap().is_none());
        assert!(parser.feed(&marker(nonce, "RESUMED")).unwrap().is_none());
        assert!(parser.feed(&vec![b'x'; MAX_STARTUP_BYTES]).is_err());
        assert!(startup_command("relative", nonce).is_err());
        assert!(startup_command("/nul\0path", nonce).is_err());
    }

    // SDTEST-2730 — execute the actual POSIX script against a disposable local
    // fixture; this is no remote connection or user shell/profile execution.
    #[cfg(unix)]
    #[tokio::test]
    async fn folder_script_holds_exact_directory_until_resume_and_restores_modes() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Stdio;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let fixture = tempfile::tempdir().unwrap();
        let folder = fixture
            .path()
            .join("folder ' \" ; $(touch INJECTED)\n trailing ");
        std::fs::create_dir(&folder).unwrap();
        let moved = fixture.path().join("moved folder ");
        let stty_log = fixture.path().join("stty.log");
        let shell_started = fixture.path().join("shell.started");
        // The test doubles only terminal mode I/O; the production command is
        // interpreted by real POSIX sh, including quoting, cd, read and exec.
        let stty = fixture.path().join("stty");
        std::fs::write(&stty, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$STTY_LOG\"\n[ \"$1\" != '-g' ] || printf 'fixture-saved-mode\\n'\n").unwrap();
        let shell = fixture.path().join("fixture-shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf 'started' > \"$SHELL_STARTED\"\npwd -P\n",
        )
        .unwrap();
        for executable in [&stty, &shell] {
            std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let nonce = "0123456789abcdef0123456789abcdef";
        let spawn = |path: &std::path::Path| {
            let mut command = tokio::process::Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(startup_command(path.to_str().unwrap(), nonce).unwrap())
                .current_dir(fixture.path())
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", fixture.path().display()),
                )
                .env("SHELL", &shell)
                .env("STTY_LOG", &stty_log)
                .env("SHELL_STARTED", &shell_started)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            command.spawn().unwrap()
        };
        async fn prepared(child: &mut tokio::process::Child, nonce: &str) {
            tokio::time::timeout(Duration::from_secs(3), async {
                let mut parser = StartupHandshake::new(nonce, "PREPARED");
                loop {
                    let byte = child.stdout.as_mut().unwrap().read_u8().await.unwrap();
                    if parser.feed(&[byte]).unwrap().is_some() {
                        break;
                    }
                }
            })
            .await
            .expect("fixture preparation must complete");
        }
        let mut child = spawn(&folder);
        prepared(&mut child, nonce).await;
        assert!(
            !shell_started.exists(),
            "prepare cannot spawn the terminal shell"
        );
        std::fs::rename(&folder, &moved).unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{nonce}\n").as_bytes())
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut resumed = StartupHandshake::new(nonce, "RESUMED");
        let bytes = resumed.feed(&result.stdout).unwrap().unwrap();
        // `pwd -P` resolves macOS /var -> /private/var and other temp-root symlinks.
        assert_eq!(
            bytes,
            format!("{}\n", moved.canonicalize().unwrap().display()).as_bytes()
        );
        assert!(shell_started.exists());
        assert!(!fixture.path().join("INJECTED").exists());
        assert_eq!(
            std::fs::read_to_string(&stty_log).unwrap(),
            "-g\n-echo\nfixture-saved-mode\n"
        );
        std::fs::remove_file(&shell_started).unwrap();
        // A mismatched control token and EOF both exit without invoking SHELL.
        for control in [Some(b"wrong-token\n".as_slice()), None] {
            let mut child = spawn(&moved);
            prepared(&mut child, nonce).await;
            if let Some(control) = control {
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(control)
                    .await
                    .unwrap();
            }
            drop(child.stdin.take());
            let result = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
            assert!(!result.status.success());
            assert!(!shell_started.exists());
        }
        let missing = spawn(&fixture.path().join("missing"));
        let result = tokio::time::timeout(Duration::from_secs(3), missing.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            result.stdout.is_empty(),
            "missing directory cannot announce readiness"
        );
        assert!(
            !shell_started.exists(),
            "missing path cannot fall back to HOME"
        );
    }
}
