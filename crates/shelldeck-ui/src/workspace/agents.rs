use super::*;
use shelldeck_core::agent_runtime::{
    configure_local_process, parse_stream_line, terminate_local_process, AgentCommandSpec,
    AgentRunRequest, AgentStreamEvent, AgentTarget, LocalProcessTree,
};
use shelldeck_ssh::client::SshClient;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

pub(super) struct ActiveAgentRun {
    shutdown_tx: tokio::sync::mpsc::Sender<()>,
    _thread: std::thread::JoinHandle<()>,
}

impl ActiveAgentRun {
    fn stop(&self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

type AgentDone = (Option<i32>, Option<String>);

impl Workspace {
    pub(super) fn handle_agent_console_event(
        &mut self,
        event: AgentConsoleEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev) {
            return;
        }
        match event {
            AgentConsoleEvent::Run(request) => self.start_agent_run(request, cx),
            AgentConsoleEvent::Stop(run_id) => self.stop_agent_run(run_id, cx),
        }
    }

    fn start_agent_run(&mut self, request: AgentRunRequest, cx: &mut Context<Self>) {
        if !self.active_agent_runs.is_empty() {
            self.show_toast(
                t!("agents.error.already_running").to_string(),
                ToastLevel::Warning,
                cx,
            );
            return;
        }
        let mut spec = match AgentCommandSpec::for_request(&request) {
            Ok(spec) => spec,
            Err(error) => {
                self.show_toast(error.to_string(), ToastLevel::Error, cx);
                return;
            }
        };
        let connection = match &request.target {
            AgentTarget::Local => None,
            AgentTarget::Ssh { connection_id, .. } => {
                let Some(connection) = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == *connection_id)
                    .cloned()
                else {
                    self.show_toast(
                        t!("agents.error.target_missing").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    return;
                };
                Some(connection)
            }
        };
        if connection.is_none() {
            if let Some(program) = resolve_local_agent_program(&spec.program) {
                spec.program = program;
            } else {
                let message = t!(
                    "agents.error.binary_missing",
                    binary = spec.program.as_str()
                )
                .to_string();
                self.agent_console
                    .update(cx, |view, cx| view.reject_run(message.clone(), cx));
                self.show_toast(message, ToastLevel::Error, cx);
                return;
            }
        }
        if connection.is_none() && !spec.cwd.is_dir() {
            let message = t!(
                "agents.error.workdir_missing",
                workdir = request.workdir.as_str()
            )
            .to_string();
            self.agent_console
                .update(cx, |view, cx| view.reject_run(message.clone(), cx));
            self.show_toast(message, ToastLevel::Error, cx);
            return;
        }

        let run_id = request.id;
        let provider = request.provider;
        self.agent_console
            .update(cx, |view, cx| view.begin_run(request.clone(), cx));

        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AgentStreamEvent>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<AgentDone>();
        let thread = match connection {
            Some(connection) => spawn_remote_agent(
                run_id,
                provider,
                spec,
                connection,
                shutdown_rx,
                event_tx,
                done_tx,
            ),
            None => spawn_local_agent(run_id, provider, spec, shutdown_rx, event_tx, done_tx),
        };
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                self.agent_console.update(cx, |view, cx| {
                    view.finish_run(run_id, None, Some(error.to_string()), cx)
                });
                return;
            }
        };
        self.active_agent_runs.insert(
            run_id,
            ActiveAgentRun {
                shutdown_tx,
                _thread: thread,
            },
        );

        let console = self.agent_console.downgrade();
        cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(50))
                .await;
            let mut events = Vec::new();
            while let Ok(event) = event_rx.try_recv() {
                events.push(event);
            }
            if !events.is_empty() {
                let _ = console.update(cx, |view, cx| {
                    for event in events {
                        view.push_stream_event(event, cx);
                    }
                });
            }
            match done_rx.try_recv() {
                Ok((exit_code, error)) => {
                    let error = error.or_else(|| {
                        (exit_code == Some(127)).then(|| {
                            t!(
                                "agents.error.remote_binary_missing",
                                binary = provider.binary()
                            )
                            .to_string()
                        })
                    });
                    let _ = console.update(cx, |view, cx| {
                        view.finish_run(run_id, exit_code, error.clone(), cx)
                    });
                    let _ = this.update(cx, |workspace, cx| {
                        workspace.active_agent_runs.remove(&run_id);
                        let level = if exit_code == Some(0) && error.is_none() {
                            ToastLevel::Success
                        } else if exit_code.is_none() && error.is_none() {
                            ToastLevel::Info
                        } else {
                            ToastLevel::Error
                        };
                        let message = if let Some(error) = error {
                            t!("agents.toast.failed", error = error).to_string()
                        } else if exit_code == Some(0) {
                            t!("agents.toast.completed").to_string()
                        } else if exit_code.is_none() {
                            t!("agents.toast.stopped").to_string()
                        } else {
                            t!(
                                "agents.toast.exit",
                                code = exit_code.unwrap_or(-1).to_string()
                            )
                            .to_string()
                        };
                        workspace.show_toast(message, level, cx);
                    });
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let _ = console.update(cx, |view, cx| {
                        view.finish_run(
                            run_id,
                            None,
                            Some(t!("agents.error.runtime_lost").to_string()),
                            cx,
                        )
                    });
                    let _ = this.update(cx, |workspace, _| {
                        workspace.active_agent_runs.remove(&run_id);
                    });
                    break;
                }
            }
        })
        .detach();
    }

    fn stop_agent_run(&mut self, run_id: Uuid, _cx: &mut Context<Self>) {
        if let Some(run) = self.active_agent_runs.get(&run_id) {
            run.stop();
        }
    }

    pub(super) fn stop_all_agent_runs(&mut self) {
        for run in self.active_agent_runs.values() {
            run.stop();
        }
        self.active_agent_runs.clear();
    }

    pub(super) fn refresh_agent_connections(&mut self, cx: &mut Context<Self>) {
        let connections = self
            .connections
            .iter()
            .map(
                |connection| crate::agent_console_view::AgentConnectionOption {
                    id: connection.id,
                    label: connection.display_name().to_string(),
                    host: connection.hostname.clone(),
                },
            )
            .collect();
        self.agent_console
            .update(cx, |view, cx| view.set_connections(connections, cx));
    }
}

fn resolve_local_agent_program(program: &str) -> Option<String> {
    if shelldeck_core::util::executable_on_path(program) {
        return Some(program.to_string());
    }
    let home = shelldeck_core::util::home_dir()?;
    let mut directories = vec![home.join(".local/bin"), home.join("bin")];
    #[cfg(unix)]
    directories.extend(["/usr/local/bin".into(), "/opt/homebrew/bin".into()]);
    #[cfg(windows)]
    let extensions = ["", ".exe", ".cmd", ".bat"];
    #[cfg(not(windows))]
    let extensions = [""];

    for directory in directories {
        for extension in extensions {
            let candidate = directory.join(format!("{program}{extension}"));
            let Ok(metadata) = candidate.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if metadata.permissions().mode() & 0o111 == 0 {
                    continue;
                }
            }
            return Some(candidate.display().to_string());
        }
    }
    None
}

fn complete_stream_lines(pending: &mut Vec<u8>, bytes: &[u8]) -> Vec<String> {
    pending.extend_from_slice(bytes);
    let mut lines = Vec::new();
    while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
        let mut line = pending[..newline].to_vec();
        pending.drain(..=newline);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        lines.push(String::from_utf8_lossy(&line).into_owned());
    }
    lines
}

fn spawn_local_agent(
    run_id: Uuid,
    provider: shelldeck_core::agent_runtime::AgentProvider,
    spec: AgentCommandSpec,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("agent-local-{run_id}"))
        .spawn(move || {
            let mut command = Command::new(&spec.program);
            command
                .args(&spec.args)
                .current_dir(&spec.cwd)
                .stdin(if spec.stdin.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            for key in &spec.remove_env {
                command.env_remove(key);
            }
            configure_local_process(&mut command);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            };
            let mut process_tree = match LocalProcessTree::capture(child.id()) {
                Ok(process_tree) => process_tree,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            };
            if let (Some(input), Some(mut stdin)) = (spec.stdin, child.stdin.take()) {
                if let Err(error) = stdin.write_all(&input) {
                    let _ = terminate_local_process(&mut child, &mut process_tree);
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            }

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let stdout_tx = event_tx.clone();
            let stdout_thread = std::thread::spawn(move || {
                if let Some(stdout) = stdout {
                    for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
                        for event in parse_stream_line(provider, &line) {
                            let _ = stdout_tx.send(event);
                        }
                    }
                }
            });
            let stderr_thread = std::thread::spawn(move || {
                if let Some(stderr) = stderr {
                    for line in BufReader::new(stderr).lines().map_while(|line| line.ok()) {
                        let _ = event_tx.send(AgentStreamEvent::Activity(line));
                    }
                }
            });

            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        process_tree.terminate();
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        let _ = done_tx.send((status.code(), None));
                        return;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = terminate_local_process(&mut child, &mut process_tree);
                        let _ = done_tx.send((None, Some(error.to_string())));
                        return;
                    }
                }
                match shutdown_rx.try_recv() {
                    Ok(()) | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        let _ = terminate_local_process(&mut child, &mut process_tree);
                        let _ = stdout_thread.join();
                        let _ = stderr_thread.join();
                        let _ = done_tx.send((None, None));
                        return;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })
}

fn spawn_remote_agent(
    run_id: Uuid,
    provider: shelldeck_core::agent_runtime::AgentProvider,
    spec: AgentCommandSpec,
    connection: Connection,
    shutdown_rx: tokio::sync::mpsc::Receiver<()>,
    event_tx: std::sync::mpsc::Sender<AgentStreamEvent>,
    done_tx: std::sync::mpsc::Sender<AgentDone>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(format!("agent-remote-{run_id}"))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = done_tx.send((None, Some(error.to_string())));
                    return;
                }
            };
            runtime.block_on(async move {
                let mut shutdown_rx = shutdown_rx;
                let client = SshClient::new();
                let connect = client.connect(&connection);
                let session = match tokio::select! {
                    _ = shutdown_rx.recv() => {
                        let _ = done_tx.send((None, None));
                        return;
                    }
                    result = connect => result,
                } {
                    Ok(session) => session,
                    Err(error) => {
                        let _ = done_tx.send((None, Some(error.to_string())));
                        return;
                    }
                };
                let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
                let forward = tokio::spawn(async move {
                    let mut pending = Vec::new();
                    while let Some(bytes) = output_rx.recv().await {
                        for line in complete_stream_lines(&mut pending, &bytes) {
                            for event in parse_stream_line(provider, &line) {
                                let _ = event_tx.send(event);
                            }
                        }
                    }
                    if !pending.is_empty() {
                        let pending = String::from_utf8_lossy(&pending);
                        if !pending.trim().is_empty() {
                            for event in parse_stream_line(provider, pending.trim()) {
                                let _ = event_tx.send(event);
                            }
                        }
                    }
                });
                let result = session
                    .exec_cancellable(&spec.remote_shell_command(), output_tx, shutdown_rx)
                    .await;
                let _ = forward.await;
                match result {
                    Ok(exit_code) => {
                        let _ = done_tx.send((exit_code.map(|code| code as i32), None));
                    }
                    Err(error) => {
                        let _ = done_tx.send((None, Some(error.to_string())));
                    }
                }
            });
        })
}

#[cfg(test)]
mod tests {
    use super::{complete_stream_lines, spawn_local_agent, AgentCommandSpec, AgentStreamEvent};
    use std::time::Duration;
    use uuid::Uuid;

    // SDTEST-1676 — SDUC-475
    #[cfg(unix)]
    #[test]
    fn sdtest_1676_local_runner_streams_and_stops_the_process_group() {
        let spec = AgentCommandSpec {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "printf '%s\\n' '{\"type\":\"content.delta\",\"delta\":\"ready\"}'; sleep 30"
                    .to_string(),
            ],
            cwd: std::env::temp_dir(),
            stdin: None,
            remove_env: Vec::new(),
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let thread = spawn_local_agent(
            Uuid::new_v4(),
            shelldeck_core::agent_runtime::AgentProvider::DeepSeek,
            spec,
            shutdown_rx,
            event_tx,
            done_tx,
        )
        .unwrap();

        assert_eq!(
            event_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            AgentStreamEvent::TextDelta("ready".to_string())
        );
        shutdown_tx.try_send(()).unwrap();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            (None, None)
        );
        thread.join().unwrap();
    }

    // SDTEST-1681 — SDUC-475
    #[test]
    fn sdtest_1681_remote_stream_reassembles_split_utf8_and_json_lines() {
        let payload = "{\"type\":\"content.delta\",\"delta\":\"terminé ✅\"}\r\n".as_bytes();
        let split = payload
            .windows(2)
            .position(|pair| pair[0] == 0xc3 && pair[1] == 0xa9)
            .unwrap()
            + 1;
        let mut pending = Vec::new();
        assert!(complete_stream_lines(&mut pending, &payload[..split]).is_empty());
        assert_eq!(
            complete_stream_lines(&mut pending, &payload[split..]),
            vec!["{\"type\":\"content.delta\",\"delta\":\"terminé ✅\"}".to_string()]
        );
        assert!(pending.is_empty());
    }
}
