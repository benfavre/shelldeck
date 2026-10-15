//! Bind controls to the PTY of one attached client, never to a guessed session.
use crate::session::{ExecResult, SshSession};
use crate::{Result, SshError};
use shelldeck_core::tmux::TmuxAction;

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxClient {
    tty: String,
}

impl TmuxClient {
    pub fn from_tty(tty: &str) -> Result<Self> {
        if !tty.starts_with("/dev/")
            || tty.len() > 256
            || tty.len() <= 5
            || !tty
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b))
        {
            return Err(SshError::Channel("Invalid tmux client identity".into()));
        }
        Ok(Self { tty: tty.into() })
    }

    pub fn command(&self, action: TmuxAction) -> String {
        let operation = match action {
            TmuxAction::SplitHorizontal => "split-window -h -t \"$pane\" -c '#{pane_current_path}'",
            TmuxAction::SplitVertical => "split-window -v -t \"$pane\" -c '#{pane_current_path}'",
            TmuxAction::NextPane => "select-pane -t \"$session:.+\"",
            TmuxAction::NextLayout => "select-layout -n -t \"$pane\"",
            TmuxAction::ToggleZoom => "resize-pane -Z -t \"$pane\"",
        };
        // Resolve the client's current session on every action: switching a tmux
        // session/window must not leave a toolbar targeting its previous pane.
        let script = format!(
            r#"set -eu
client={}
session=$(tmux list-clients -F '#{{client_tty}} #{{session_id}}' | awk -v client="$client" '$1 == client {{ print $2 }}')
case "$session" in '$'*) ;; *) echo 'tmux client is no longer attached' >&2; exit 1 ;; esac
number=${{session#\$}}
case "$number" in ''|*[!0-9]*) echo 'Invalid tmux session identity' >&2; exit 1 ;; esac
pane=$(tmux display-message -p -t "$session:" '#{{pane_id}}')
case "$pane" in '%'* ) ;; *) echo 'No active tmux pane' >&2; exit 1 ;; esac
exec tmux {operation}
"#,
            shell_quote(&self.tty)
        );
        format!("sh -c {}", shell_quote(&script))
    }

    pub async fn apply(&self, executor: &impl TmuxExecutor, action: TmuxAction) -> Result<()> {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            executor.execute_tmux(&self.command(action)),
        )
        .await
        .map_err(|_| SshError::Channel("tmux control timed out".into()))??;
        if !result.success() {
            return Err(SshError::Channel(result.stderr_string().trim().to_string()));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait TmuxExecutor: Sync {
    async fn execute_tmux(&self, command: &str) -> Result<ExecResult>;
}

#[async_trait::async_trait]
impl TmuxExecutor for SshSession {
    async fn execute_tmux(&self, command: &str) -> Result<ExecResult> {
        self.exec(command).await
    }
}

/// Consume the private startup header while preserving all following TUI bytes.
pub(crate) struct TmuxHandshake {
    marker: String,
    buffer: Vec<u8>,
}

impl TmuxHandshake {
    pub fn new() -> Self {
        Self {
            marker: format!("SHELLDECK_TMUX_{}:", uuid::Uuid::new_v4().simple()),
            buffer: Vec::new(),
        }
    }

    pub fn startup_command(&self) -> String {
        let script = format!(
            "command -v tmux >/dev/null || {{ echo 'tmux is not installed' >&2; exit 127; }}; printf '%s%s\\n' {} \"$(tty)\"; exec tmux new-session -A -s main",
            shell_quote(&self.marker)
        );
        format!("sh -c {}", shell_quote(&script))
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Option<(TmuxClient, Vec<u8>)>> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > 65536 {
            return Err(SshError::Channel(
                "tmux startup header exceeded its limit".into(),
            ));
        }
        let Some(start) = self
            .buffer
            .windows(self.marker.len())
            .position(|w| w == self.marker.as_bytes())
        else {
            return Ok(None);
        };
        let tty_start = start + self.marker.len();
        let Some(end) = self.buffer[tty_start..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| tty_start + i)
        else {
            return Ok(None);
        };
        let tty = std::str::from_utf8(&self.buffer[tty_start..end])
            .map_err(|_| SshError::Channel("Invalid tmux startup encoding".into()))?
            .trim_end_matches('\r');
        let client = TmuxClient::from_tty(tty)?;
        Ok(Some((client, self.buffer[end + 1..].to_vec())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // SDTEST-1951 — SDUC-500: arbitrary SSH chunking cannot corrupt the TUI.
    #[test]
    fn tmux_handshake_accepts_fragmented_headers_and_preserves_terminal_output() {
        let mut header = TmuxHandshake::new();
        let wire = format!(
            "login banner\r\n{}/dev/pts/17\r\n\x1b[?1049hhello",
            header.marker
        );
        let split = wire.find("\x1b").unwrap() - 1;
        for byte in &wire.as_bytes()[..split] {
            assert!(header.feed(&[*byte]).unwrap().is_none());
        }
        let (client, output) = header.feed(&wire.as_bytes()[split..]).unwrap().unwrap();
        assert_eq!(client.tty, "/dev/pts/17");
        assert_eq!(output, b"\x1b[?1049hhello");
    }

    // SDTEST-1952 — SDUC-500: identities cannot become shell commands.
    #[test]
    fn tmux_handshake_rejects_invalid_identity_and_unbounded_preludes() {
        for tty in [
            "/dev/pts/1;touch /tmp/no",
            "-t main",
            "",
            "/dev/pts/1\nother",
        ] {
            assert!(TmuxClient::from_tty(tty).is_err());
        }
        let mut header = TmuxHandshake::new();
        assert!(header.feed(&vec![b'x'; 65537]).is_err());
    }

    struct FakeExecutor {
        commands: Mutex<Vec<String>>,
        success: bool,
    }
    #[async_trait::async_trait]
    impl TmuxExecutor for FakeExecutor {
        async fn execute_tmux(&self, command: &str) -> Result<ExecResult> {
            self.commands.lock().unwrap().push(command.to_owned());
            Ok(ExecResult {
                stdout: Vec::new(),
                stderr: if self.success {
                    Vec::new()
                } else {
                    b"detached client".to_vec()
                },
                exit_code: Some(if self.success { 0 } else { 1 }),
            })
        }
    }

    // SDTEST-1953 — SDUC-500: one exact-client command, no keyboard injection.
    #[tokio::test]
    async fn tmux_controls_use_the_attached_client_and_propagate_failures_without_fallback() {
        let client = TmuxClient::from_tty("/dev/pts/17").unwrap();
        let fake = FakeExecutor {
            commands: Mutex::new(Vec::new()),
            success: false,
        };
        let error = client
            .apply(&fake, TmuxAction::SplitHorizontal)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("detached client"));
        let commands = fake.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0], client.command(TmuxAction::SplitHorizontal));
        assert!(commands[0].contains("client_tty"));
        assert!(commands[0].contains("/dev/pts/17"));
        assert!(commands[0].contains("pane_current_path"));
        assert!(!commands[0].contains("send-keys"));
        assert!(!commands[0].contains("-s main"));
    }
}
