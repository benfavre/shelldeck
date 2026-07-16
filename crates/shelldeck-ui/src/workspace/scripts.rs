use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{create_client, parse_generated_script_draft, AiContext, AiSurface};
use shelldeck_core::config::activity::{ActivityAction, ActivityEntry, ActivityKind};
use shelldeck_core::models::connection::Connection;
use shelldeck_core::models::script::{ScriptLanguage, ScriptTarget};
use shelldeck_core::models::script_runner::build_command;
use shelldeck_core::models::templates::all_templates;
use shelldeck_ssh::client::SshClient;
use uuid::Uuid;

use crate::script_editor::ScriptEvent;
use crate::script_form::{ScriptForm, ScriptFormEvent};
use crate::t;
use crate::template_browser::{TemplateBrowser, TemplateBrowserEvent};
use crate::terminal_view::PinnedScript;
use crate::toast::ToastLevel;
use crate::variable_prompt::{VariablePrompt, VariablePromptEvent};

use super::{ActiveScript, Workspace};

impl Workspace {
    pub(super) fn handle_script_event(&mut self, event: &ScriptEvent, cx: &mut Context<Self>) {
        match event {
            ScriptEvent::RunScript(script) => {
                // Guard: don't start if already running
                if self.scripts.read(cx).is_running() {
                    self.show_toast(
                        t!("toast.script.already_running").to_string(),
                        ToastLevel::Warning,
                        cx,
                    );
                    return;
                }

                // Check for template variables — show prompt if any exist
                let resolved = script.resolved_variables();
                if !resolved.is_empty() {
                    self.show_variable_prompt(script.clone(), resolved, cx);
                    return;
                }

                tracing::info!("Running script: {}", script.name);
                let cmd = build_command(script, None);
                let script_name = script.name.clone();
                let script_id = script.id;
                let connection_id = match &script.target {
                    ScriptTarget::Remote(cid) => Some(*cid),
                    _ => None,
                };

                // Create execution record
                let record = shelldeck_core::models::execution::ExecutionRecord::new(
                    script_id,
                    connection_id,
                );

                let display_cmd = if matches!(script.language, ScriptLanguage::Shell) {
                    format!("$ {}", script.body)
                } else {
                    format!("$ [{}] {}", script.language.label(), cmd.ssh_command)
                };

                self.scripts.update(cx, |editor, _| {
                    editor.running_script_id = Some(script_id);
                    editor.execution_output.clear();
                    editor.execution_output.push(display_cmd);
                    editor.history.push(record);
                });

                // Update last_run / run_count on the script
                self.scripts.update(cx, |editor, _| {
                    if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == script_id) {
                        s.last_run = Some(chrono::Utc::now());
                        s.run_count += 1;
                    }
                });
                // Persist run stats to store
                if let Some(s) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == script_id)
                    .cloned()
                {
                    let _ = self.store.update_script(s);
                }

                self.add_activity_entry(
                    ActivityEntry::new(
                        ActivityKind::Script,
                        t!("activity.script.running", name = script_name.as_str()).to_string(),
                    )
                    .with_target(script_id.to_string(), script_name.clone())
                    .with_action(ActivityAction::OpenScript),
                    cx,
                );
                self.show_toast(
                    t!("toast.script.running", name = script_name.as_str()).to_string(),
                    ToastLevel::Info,
                    cx,
                );
                self.update_dashboard_stats(cx);

                // Route based on script target
                match &script.target {
                    ScriptTarget::Remote(connection_id) => {
                        let connection = self
                            .connections
                            .iter()
                            .find(|c| c.id == *connection_id)
                            .cloned();
                        if let Some(conn) = connection {
                            self.run_script_remote(
                                cmd.ssh_command.clone(),
                                script_name,
                                script_id,
                                conn,
                                cx,
                            );
                        } else {
                            tracing::error!(
                                "Connection {} not found for remote script",
                                connection_id
                            );
                            self.scripts.update(cx, |editor, cx| {
                                editor.running_script_id = None;
                                editor
                                    .execution_output
                                    .push(format!("Error: Connection {} not found", connection_id));
                                cx.notify();
                            });
                            self.show_toast(
                                t!("toast.script.remote_not_found").to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                            self.update_dashboard_stats(cx);
                        }
                    }
                    ScriptTarget::Local | ScriptTarget::AskOnRun => {
                        self.run_script_local_cmd(
                            cmd.local_binary.clone(),
                            cmd.local_args.clone(),
                            cmd.env_vars.clone(),
                            script_name,
                            script_id,
                            cx,
                        );
                    }
                }

                // Sync favorites/recent to terminal toolbar
                self.sync_scripts_to_terminal_toolbar(cx);

                cx.notify();
            }
            ScriptEvent::StopScript => {
                let script_id = self.scripts.read(cx).running_script_id;
                if let Some(sid) = script_id {
                    if let Some(active) = self.active_scripts.remove(&sid) {
                        active.stop();
                    }

                    self.scripts.update(cx, |editor, cx| {
                        editor.running_script_id = None;
                        editor
                            .execution_output
                            .push("[Script cancelled]".to_string());
                        // Finalize the last execution record
                        if let Some(record) = editor.history.last_mut() {
                            record.finish(-1);
                        }
                        cx.notify();
                    });

                    self.show_toast(
                        t!("toast.script.cancelled").to_string(),
                        ToastLevel::Info,
                        cx,
                    );
                    self.update_dashboard_stats(cx);
                    cx.notify();
                }
            }
            ScriptEvent::AddScript => {
                self.show_script_form(cx);
            }
            ScriptEvent::EditScript(id) => {
                if let Some(script) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == *id)
                    .cloned()
                {
                    self.show_script_form_edit(&script, cx);
                }
            }
            ScriptEvent::UpdateScript(script) => {
                tracing::info!("Script updated (inline): {}", script.name);
                // Update in store
                match self.store.update_script(script.clone()) {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Err(e) = self.store.add_script(script.clone()) {
                            tracing::error!("Failed to save script: {}", e);
                            self.show_toast(
                                t!("toast.script.save_failed", error = e.to_string()).to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to update script: {}", e);
                        self.show_toast(
                            t!("toast.script.update_failed", error = e.to_string()).to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                }
                // Update in script editor view
                self.scripts.update(cx, |editor, _| {
                    if let Some(existing) = editor.scripts.iter_mut().find(|s| s.id == script.id) {
                        *existing = script.clone();
                    }
                });
                self.add_activity_entry(
                    ActivityEntry::new(
                        ActivityKind::Script,
                        t!("activity.script.updated", name = script.name.as_str()).to_string(),
                    )
                    .with_target(script.id.to_string(), script.name.clone())
                    .with_action(ActivityAction::OpenScript),
                    cx,
                );
                self.show_toast(
                    t!("toast.script.updated", name = script.name.as_str()).to_string(),
                    ToastLevel::Success,
                    cx,
                );
                cx.notify();
            }
            ScriptEvent::ClearOutput => {}
            ScriptEvent::ToggleFavorite(id) => {
                let id = *id;
                self.scripts.update(cx, |editor, _| {
                    if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == id) {
                        s.is_favorite = !s.is_favorite;
                    }
                });
                if let Some(s) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .cloned()
                {
                    let _ = self.store.update_script(s);
                }
                self.sync_scripts_to_terminal_toolbar(cx);
                cx.notify();
            }
            ScriptEvent::TogglePinToToolbar(id) => {
                let id = *id;
                self.scripts.update(cx, |editor, _| {
                    if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == id) {
                        s.pinned_to_toolbar = !s.pinned_to_toolbar;
                    }
                });
                if let Some(s) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .cloned()
                {
                    let _ = self.store.update_script(s);
                }
                self.sync_scripts_to_terminal_toolbar(cx);
                cx.notify();
            }
            ScriptEvent::DeleteScript(id) => {
                let id = *id;
                let name = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.clone());
                self.scripts.update(cx, |editor, _| {
                    editor.scripts.retain(|s| s.id != id);
                    if editor.selected_script == Some(id) {
                        editor.selected_script = None;
                    }
                });
                let _ = self.store.remove_script(id);
                if let Some(name) = name {
                    self.show_toast(
                        t!("toast.script.deleted", name = name.as_str()).to_string(),
                        ToastLevel::Info,
                        cx,
                    );
                }
                self.sync_scripts_to_terminal_toolbar(cx);
                cx.notify();
            }
            ScriptEvent::ImportTemplate(template_id) => {
                let template_id = template_id.clone();
                if let Some(tmpl) = all_templates().iter().find(|t| t.id == template_id) {
                    let script = tmpl.to_script();
                    let name = script.name.clone();
                    self.scripts.update(cx, |editor, _| {
                        editor.scripts.push(script.clone());
                    });
                    let _ = self.store.add_script(script);
                    self.show_toast(
                        t!("toast.script.imported_template", name = name.as_str()).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    self.sync_scripts_to_terminal_toolbar(cx);
                    cx.notify();
                }
            }
            ScriptEvent::RunScriptById(id) => {
                let id = *id;
                if let Some(script) = self
                    .scripts
                    .read(cx)
                    .scripts
                    .iter()
                    .find(|s| s.id == id)
                    .cloned()
                {
                    self.handle_script_event(&ScriptEvent::RunScript(script), cx);
                }
            }
            ScriptEvent::GenerateWithAi(id) => {
                self.open_ai_workflow(
                    super::AiWorkflowTarget::ScriptGenerate {
                        script_id: id.to_string(),
                    },
                    cx,
                );
            }
            ScriptEvent::ExplainWithAi(id) => {
                self.open_ai_workflow(
                    super::AiWorkflowTarget::ScriptExplain {
                        script_id: id.to_string(),
                    },
                    cx,
                );
            }
            ScriptEvent::ReviewWithAi(id) => {
                self.open_ai_workflow(
                    super::AiWorkflowTarget::ScriptReview {
                        script_id: id.to_string(),
                    },
                    cx,
                );
            }
        }
    }

    pub(super) fn run_script_local_cmd(
        &mut self,
        binary: String,
        args: Vec<String>,
        env_vars: Vec<(String, String)>,
        script_name: String,
        script_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<String>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Option<i32>>();

        let thread_handle = std::thread::Builder::new()
            .name(format!("script-local-{}", script_id))
            .spawn(move || {
                let mut cmd = Command::new(&binary);
                cmd.args(&args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                for (k, v) in &env_vars {
                    cmd.env(k, v);
                }
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = stream_tx.send(format!("Error: {}", e));
                        let _ = done_tx.send(None);
                        return;
                    }
                };

                // Spawn reader threads for stdout and stderr
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let stream_tx2 = stream_tx.clone();

                let stdout_thread = std::thread::spawn(move || {
                    if let Some(stdout) = stdout {
                        for line in BufReader::new(stdout).lines() {
                            match line {
                                Ok(l) => {
                                    let _ = stream_tx.send(l);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                });

                let stderr_thread = std::thread::spawn(move || {
                    if let Some(stderr) = stderr {
                        let mut first = true;
                        for line in BufReader::new(stderr).lines() {
                            match line {
                                Ok(l) => {
                                    if first {
                                        let _ = stream_tx2.send("--- stderr ---".to_string());
                                        first = false;
                                    }
                                    let _ = stream_tx2.send(l);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                });

                // Create a blocking receiver for the shutdown signal
                let mut shutdown_rx = shutdown_rx;

                // Poll for completion or cancellation
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(status.code());
                            return;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(None);
                            tracing::error!("Error waiting for child process: {}", e);
                            return;
                        }
                    }

                    // Check shutdown (non-blocking)
                    match shutdown_rx.try_recv() {
                        Ok(()) => {
                            // Kill the child process
                            let _ = child.kill();
                            let _ = child.wait(); // reap
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(Some(-1));
                            return;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            // Sender dropped — treat as cancellation
                            let _ = child.kill();
                            let _ = child.wait();
                            let _ = stdout_thread.join();
                            let _ = stderr_thread.join();
                            let _ = done_tx.send(Some(-1));
                            return;
                        }
                    }

                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            });
        let thread_handle = match thread_handle {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to spawn local script thread: {}", e);
                self.show_toast(
                    t!("toast.script.start_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };

        self.active_scripts.insert(
            script_id,
            ActiveScript {
                shutdown_tx,
                _thread: Some(thread_handle),
            },
        );

        // UI poller: drains output and handles completion
        let scripts_handle = self.scripts.downgrade();
        let script_name_done = script_name;
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                // Drain output lines
                let mut lines = Vec::new();
                while let Ok(line) = stream_rx.try_recv() {
                    lines.push(line);
                }

                if !lines.is_empty() {
                    let _ = scripts_handle.update(cx, |editor, cx| {
                        for line in &lines {
                            editor.execution_output.push(line.clone());
                        }
                        // Also append to execution record
                        if let Some(record) = editor.history.last_mut() {
                            for line in &lines {
                                record.append_output(line);
                                record.append_output("\n");
                            }
                        }
                        cx.notify();
                    });
                }

                // Check if done
                match done_rx.try_recv() {
                    Ok(exit_code) => {
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            let code = exit_code.unwrap_or(-1);
                            editor.execution_output.push(format!("Exit code: {}", code));
                            // Finalize execution record
                            if let Some(record) = editor.history.last_mut() {
                                record.finish(code);
                            }
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                            let code = exit_code.unwrap_or(-1);
                            let (activity_message, level) = match exit_code {
                                Some(0) => (
                                    t!(
                                        "activity.script.completed",
                                        name = script_name_done.as_str()
                                    )
                                    .to_string(),
                                    ToastLevel::Success,
                                ),
                                Some(code) => (
                                    t!(
                                        "activity.script.exited",
                                        name = script_name_done.as_str(),
                                        code = code
                                    )
                                    .to_string(),
                                    ToastLevel::Error,
                                ),
                                None => (
                                    t!("activity.script.failed", name = script_name_done.as_str())
                                        .to_string(),
                                    ToastLevel::Error,
                                ),
                            };
                            ws.add_activity_entry(
                                ActivityEntry::new(ActivityKind::Script, activity_message)
                                    .with_target(script_id.to_string(), script_name_done.clone())
                                    .with_detail(
                                        t!("activity.script.exit_code", code = code).to_string(),
                                    )
                                    .with_action(ActivityAction::OpenScript),
                                cx,
                            );
                            match exit_code {
                                Some(0) => {
                                    ws.show_toast(
                                        t!(
                                            "toast.script.completed",
                                            name = script_name_done.as_str()
                                        )
                                        .to_string(),
                                        level,
                                        cx,
                                    );
                                }
                                Some(code) => {
                                    ws.show_toast(
                                        t!(
                                            "toast.script.exited_with_code",
                                            name = script_name_done.as_str(),
                                            code = code
                                        )
                                        .to_string(),
                                        level,
                                        cx,
                                    );
                                }
                                None => {
                                    ws.show_toast(
                                        t!(
                                            "toast.script.failed_to_execute",
                                            name = script_name_done.as_str()
                                        )
                                        .to_string(),
                                        level,
                                        cx,
                                    );
                                }
                            }
                        });
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Thread exited without sending done — clean up
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            editor
                                .execution_output
                                .push("[Script thread exited unexpectedly]".to_string());
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    pub(super) fn run_script_remote(
        &mut self,
        body: String,
        script_name: String,
        script_id: Uuid,
        connection: Connection,
        cx: &mut Context<Self>,
    ) {
        let host_display = connection.display_name().to_string();

        self.scripts.update(cx, |editor, _| {
            editor
                .execution_output
                .push(format!("[remote: {}]", host_display));
        });

        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let (stream_tx, stream_rx) = std::sync::mpsc::channel::<String>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Option<i32>>();

        let thread_handle = std::thread::Builder::new()
            .name(format!("script-remote-{}", script_id))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ =
                            stream_tx.send(format!("Error: failed to create async runtime: {}", e));
                        let _ = done_tx.send(None);
                        return;
                    }
                };

                rt.block_on(async move {
                    let client = SshClient::new();
                    let session = match client.connect(&connection).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = stream_tx.send(format!("Error: SSH connection failed: {}", e));
                            let _ = done_tx.send(None);
                            return;
                        }
                    };

                    // Create channel for exec_cancellable output
                    let (output_tx, mut output_rx) =
                        tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

                    // Forward tokio output_rx -> std stream_tx
                    let fwd_stream_tx = stream_tx.clone();
                    let fwd_task = tokio::spawn(async move {
                        while let Some(data) = output_rx.recv().await {
                            let text = String::from_utf8_lossy(&data);
                            for line in text.lines() {
                                let _ = fwd_stream_tx.send(line.to_string());
                            }
                        }
                    });

                    let result = session
                        .exec_cancellable(&body, output_tx, shutdown_rx)
                        .await;

                    // Wait for forwarding to flush
                    let _ = fwd_task.await;

                    match result {
                        Ok(exit_code) => {
                            let _ = done_tx.send(exit_code.map(|c| c as i32));
                        }
                        Err(e) => {
                            let _ = stream_tx.send(format!("Error: {}", e));
                            let _ = done_tx.send(None);
                        }
                    }
                });
            });
        let thread_handle = match thread_handle {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to spawn remote script thread: {}", e);
                self.show_toast(
                    t!("toast.script.start_remote_failed", error = e.to_string()).to_string(),
                    ToastLevel::Error,
                    cx,
                );
                return;
            }
        };

        self.active_scripts.insert(
            script_id,
            ActiveScript {
                shutdown_tx,
                _thread: Some(thread_handle),
            },
        );

        // UI poller
        let scripts_handle = self.scripts.downgrade();
        let script_name_done = script_name;
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;

                // Drain output lines
                let mut lines = Vec::new();
                while let Ok(line) = stream_rx.try_recv() {
                    lines.push(line);
                }

                if !lines.is_empty() {
                    let _ = scripts_handle.update(cx, |editor, cx| {
                        for line in &lines {
                            editor.execution_output.push(line.clone());
                        }
                        if let Some(record) = editor.history.last_mut() {
                            for line in &lines {
                                record.append_output(line);
                                record.append_output("\n");
                            }
                        }
                        cx.notify();
                    });
                }

                // Check if done
                match done_rx.try_recv() {
                    Ok(exit_code) => {
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            let code = exit_code.unwrap_or(-1);
                            editor.execution_output.push(format!("Exit code: {}", code));
                            if let Some(record) = editor.history.last_mut() {
                                record.finish(code);
                            }
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                            let code = exit_code.unwrap_or(-1);
                            let activity_message = match exit_code {
                                Some(0) | None => t!(
                                    "activity.script.completed_on",
                                    name = script_name_done.as_str(),
                                    host = host_display.as_str()
                                )
                                .to_string(),
                                Some(code) => t!(
                                    "activity.script.exited_on",
                                    name = script_name_done.as_str(),
                                    code = code,
                                    host = host_display.as_str()
                                )
                                .to_string(),
                            };
                            ws.add_activity_entry(
                                ActivityEntry::new(ActivityKind::Script, activity_message)
                                    .with_target(script_id.to_string(), script_name_done.clone())
                                    .with_detail(
                                        t!("activity.script.exit_code", code = code).to_string(),
                                    )
                                    .with_action(ActivityAction::OpenScript),
                                cx,
                            );
                            match exit_code {
                                Some(0) | None => {
                                    ws.show_toast(
                                        t!(
                                            "toast.script.completed_on",
                                            name = script_name_done.as_str(),
                                            host = host_display.as_str()
                                        )
                                        .to_string(),
                                        ToastLevel::Success,
                                        cx,
                                    );
                                }
                                Some(code) => {
                                    ws.show_toast(
                                        t!(
                                            "toast.script.exited_on",
                                            name = script_name_done.as_str(),
                                            code = code,
                                            host = host_display.as_str()
                                        )
                                        .to_string(),
                                        ToastLevel::Error,
                                        cx,
                                    );
                                }
                            }
                        });
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let _ = scripts_handle.update(cx, |editor, cx| {
                            editor.running_script_id = None;
                            editor
                                .execution_output
                                .push("[Remote script thread exited unexpectedly]".to_string());
                            cx.notify();
                        });
                        let _ = _this.update(cx, |ws, cx| {
                            ws.active_scripts.remove(&script_id);
                            ws.update_dashboard_stats(cx);
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// Push favorite and recent scripts to the terminal toolbar.
    pub(super) fn sync_scripts_to_terminal_toolbar(&self, cx: &mut Context<Self>) {
        let scripts = &self.scripts.read(cx).scripts;

        let favorites: Vec<(Uuid, String, ScriptLanguage)> = scripts
            .iter()
            .filter(|s| s.is_favorite)
            .map(|s| (s.id, s.name.clone(), s.language.clone()))
            .collect();

        let mut recent: Vec<_> = scripts
            .iter()
            .filter(|s| s.last_run.is_some())
            .collect::<Vec<_>>();
        recent.sort_by_key(|s| std::cmp::Reverse(s.last_run));
        let recent: Vec<(Uuid, String, ScriptLanguage)> = recent
            .into_iter()
            .take(5)
            .map(|s| (s.id, s.name.clone(), s.language.clone()))
            .collect();

        let pinned: Vec<PinnedScript> = scripts
            .iter()
            .filter(|s| s.pinned_to_toolbar)
            .map(|s| PinnedScript {
                id: s.id,
                name: s.name.clone(),
                badge: s.language.badge().to_string(),
                badge_color: s.language.badge_color(),
            })
            .collect();

        self.terminal.update(cx, |tv, _| {
            tv.set_scripts(favorites, recent);
            tv.set_pinned_scripts(pinned);
        });
    }

    pub(super) fn show_template_browser(&mut self, cx: &mut Context<Self>) {
        let browser = cx.new(TemplateBrowser::new);

        let sub = cx.subscribe(
            &browser,
            |this, _browser, event: &TemplateBrowserEvent, cx| match event {
                TemplateBrowserEvent::Import(script) => {
                    let name = script.name.clone();
                    this.scripts.update(cx, |editor, _| {
                        editor.scripts.push(script.clone());
                    });
                    let _ = this.store.add_script(script.clone());
                    this.show_toast(
                        t!("toast.script.imported_template", name = name.as_str()).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    this.sync_scripts_to_terminal_toolbar(cx);
                    this.template_browser = None;
                    this._template_browser_sub = None;
                    cx.notify();
                }
                TemplateBrowserEvent::Cancel => {
                    this.template_browser = None;
                    this._template_browser_sub = None;
                    cx.notify();
                }
            },
        );

        self.template_browser = Some(browser);
        self._template_browser_sub = Some(sub);
        cx.notify();
    }

    pub(super) fn show_variable_prompt(
        &mut self,
        script: shelldeck_core::models::script::Script,
        variables: Vec<shelldeck_core::models::script::ScriptVariable>,
        cx: &mut Context<Self>,
    ) {
        let script_clone = script.clone();
        let prompt = cx.new(|cx| VariablePrompt::new(script_clone, variables, cx));

        let sub = cx.subscribe(
            &prompt,
            |this, _prompt, event: &VariablePromptEvent, cx| match event {
                VariablePromptEvent::Run(script, values) => {
                    this.variable_prompt = None;
                    this._variable_prompt_sub = None;
                    this.run_script_with_values(script.clone(), values.clone(), cx);
                    cx.notify();
                }
                VariablePromptEvent::Cancel => {
                    this.variable_prompt = None;
                    this._variable_prompt_sub = None;
                    cx.notify();
                }
            },
        );

        self.variable_prompt = Some(prompt);
        self._variable_prompt_sub = Some(sub);
        cx.notify();
    }

    pub(super) fn run_script_with_values(
        &mut self,
        script: shelldeck_core::models::script::Script,
        values: std::collections::HashMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        tracing::info!("Running script with variables: {}", script.name);
        let cmd = build_command(&script, Some(&values));
        let script_name = script.name.clone();
        let script_id = script.id;
        let connection_id = match &script.target {
            ScriptTarget::Remote(cid) => Some(*cid),
            _ => None,
        };

        let record =
            shelldeck_core::models::execution::ExecutionRecord::new(script_id, connection_id);

        let display_cmd = if matches!(script.language, ScriptLanguage::Shell) {
            format!(
                "$ {}",
                shelldeck_core::models::script_runner::substitute_variables(&script.body, &values)
            )
        } else {
            format!("$ [{}] {}", script.language.label(), cmd.ssh_command)
        };

        let values_for_store = values.clone();
        self.scripts.update(cx, |editor, _| {
            editor.running_script_id = Some(script_id);
            editor.execution_output.clear();
            editor.execution_output.push(display_cmd);
            editor.history.push(record);
            // Store the variable values for display in the variables bar
            editor.last_var_values.insert(script_id, values_for_store);
        });

        self.scripts.update(cx, |editor, _| {
            if let Some(s) = editor.scripts.iter_mut().find(|s| s.id == script_id) {
                s.last_run = Some(chrono::Utc::now());
                s.run_count += 1;
            }
        });
        if let Some(s) = self
            .scripts
            .read(cx)
            .scripts
            .iter()
            .find(|s| s.id == script_id)
            .cloned()
        {
            let _ = self.store.update_script(s);
        }

        self.add_activity_entry(
            ActivityEntry::new(
                ActivityKind::Script,
                t!("activity.script.running", name = script_name.as_str()).to_string(),
            )
            .with_target(script_id.to_string(), script_name.clone())
            .with_action(ActivityAction::OpenScript),
            cx,
        );
        self.show_toast(
            t!("toast.script.running", name = script_name.as_str()).to_string(),
            ToastLevel::Info,
            cx,
        );
        self.update_dashboard_stats(cx);

        match &script.target {
            ScriptTarget::Remote(connection_id) => {
                let connection = self
                    .connections
                    .iter()
                    .find(|c| c.id == *connection_id)
                    .cloned();
                if let Some(conn) = connection {
                    self.run_script_remote(
                        cmd.ssh_command.clone(),
                        script_name,
                        script_id,
                        conn,
                        cx,
                    );
                } else {
                    tracing::error!("Connection {} not found for remote script", connection_id);
                    self.scripts.update(cx, |editor, cx| {
                        editor.running_script_id = None;
                        editor
                            .execution_output
                            .push(format!("Error: Connection {} not found", connection_id));
                        cx.notify();
                    });
                    self.show_toast(
                        t!("toast.script.remote_not_found").to_string(),
                        ToastLevel::Error,
                        cx,
                    );
                    self.update_dashboard_stats(cx);
                }
            }
            ScriptTarget::Local | ScriptTarget::AskOnRun => {
                self.run_script_local_cmd(
                    cmd.local_binary.clone(),
                    cmd.local_args.clone(),
                    cmd.env_vars.clone(),
                    script_name,
                    script_id,
                    cx,
                );
            }
        }

        self.sync_scripts_to_terminal_toolbar(cx);
        cx.notify();
    }

    pub(super) fn show_script_form(&mut self, cx: &mut Context<Self>) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let ai_enabled =
            self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Script);
        let form = cx.new(|form_cx| ScriptForm::new(connections, ai_enabled, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &ScriptFormEvent, cx| {
            match event {
                ScriptFormEvent::Save(script) => {
                    tracing::info!("Script created: {}", script.name);
                    // Persist to store
                    if let Err(e) = this.store.add_script(script.clone()) {
                        tracing::error!("Failed to save script: {}", e);
                        this.show_toast(
                            t!("toast.script.save_failed", error = e.to_string()).to_string(),
                            ToastLevel::Error,
                            cx,
                        );
                    }
                    // Add to script editor view
                    this.scripts.update(cx, |editor, _| {
                        editor.add_script(script.clone());
                    });
                    this.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Script,
                            t!("activity.script.added", name = script.name.as_str()).to_string(),
                        )
                        .with_target(script.id.to_string(), script.name.clone())
                        .with_action(ActivityAction::OpenScript),
                        cx,
                    );
                    this.show_toast(
                        t!("toast.script.created", name = script.name.as_str()).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
                ScriptFormEvent::GenerateWithAi { instructions } => {
                    this.generate_script_form_with_ai(instructions.clone(), cx);
                }
                ScriptFormEvent::Cancel => {
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.script_form = Some(form);
        self._script_form_sub = Some(sub);
        cx.notify();
    }

    pub(super) fn show_script_form_edit(
        &mut self,
        script: &shelldeck_core::models::script::Script,
        cx: &mut Context<Self>,
    ) {
        let connections: Vec<(Uuid, String, String)> = self
            .connections
            .iter()
            .map(|c| (c.id, c.display_name().to_string(), c.hostname.clone()))
            .collect();

        let script = script.clone();
        let ai_enabled =
            self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Script);
        let form =
            cx.new(|form_cx| ScriptForm::from_script(&script, connections, ai_enabled, form_cx));

        let sub = cx.subscribe(&form, |this, _form, event: &ScriptFormEvent, cx| {
            match event {
                ScriptFormEvent::Save(script) => {
                    tracing::info!("Script updated: {}", script.name);
                    // Update in store
                    match this.store.update_script(script.clone()) {
                        Ok(true) => {}
                        Ok(false) => {
                            // Not found in store, add it
                            if let Err(e) = this.store.add_script(script.clone()) {
                                tracing::error!("Failed to save script: {}", e);
                                this.show_toast(
                                    t!("toast.script.save_failed", error = e.to_string())
                                        .to_string(),
                                    ToastLevel::Error,
                                    cx,
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to update script: {}", e);
                            this.show_toast(
                                t!("toast.script.update_failed", error = e.to_string()).to_string(),
                                ToastLevel::Error,
                                cx,
                            );
                        }
                    }
                    // Update in script editor view
                    this.scripts.update(cx, |editor, _| {
                        if let Some(existing) =
                            editor.scripts.iter_mut().find(|s| s.id == script.id)
                        {
                            *existing = script.clone();
                        }
                    });
                    this.add_activity_entry(
                        ActivityEntry::new(
                            ActivityKind::Script,
                            t!("activity.script.updated", name = script.name.as_str()).to_string(),
                        )
                        .with_target(script.id.to_string(), script.name.clone())
                        .with_action(ActivityAction::OpenScript),
                        cx,
                    );
                    this.show_toast(
                        t!("toast.script.updated", name = script.name.as_str()).to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                    // Close form
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
                ScriptFormEvent::GenerateWithAi { instructions } => {
                    this.generate_script_form_with_ai(instructions.clone(), cx);
                }
                ScriptFormEvent::Cancel => {
                    this.script_form = None;
                    this._script_form_sub = None;
                    cx.notify();
                }
            }
        });

        self.script_form = Some(form);
        self._script_form_sub = Some(sub);
        cx.notify();
    }

    fn generate_script_form_with_ai(&mut self, instructions: String, cx: &mut Context<Self>) {
        let Some(form) = self.script_form.as_ref().cloned() else {
            return;
        };
        let context = AiContext::new(
            AiSurface::Script,
            t!("ai.context.script_form").to_string(),
            serde_json::json!({
                "draft": form.read(cx).ai_context_data(cx),
                "hosts": self.ai_hosts_context_data(),
            }),
        );
        let prompt = format!(
            "{}\n\n{}:\n{}",
            t!("ai.prompt.script_generate_form"),
            t!("ai.workflow.additional_instructions"),
            instructions.trim()
        );
        let config = self.app_config.ai.clone();
        let form = form.downgrade();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = create_client(&config)?;
                    let response = client.complete(&prompt, context.clone())?;
                    match parse_generated_script_draft(&response.text) {
                        Ok(draft) => Ok(draft),
                        Err(first_error) => {
                            let repair_prompt = format!(
                                "{}\n\n{}",
                                prompt,
                                t!(
                                    "ai.prompt.script_generate_repair",
                                    error = first_error.to_string()
                                )
                            );
                            let repaired = client.complete(&repair_prompt, context)?;
                            parse_generated_script_draft(&repaired.text)
                        }
                    }
                })
                .await
                .map_err(|error| error.to_string());
            if let Some(form) = form.upgrade() {
                let _ = form.update(cx, |form, cx| form.set_ai_result(result, cx));
            }
        })
        .detach();
    }
}
