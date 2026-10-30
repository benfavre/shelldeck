//! User-selected SSH metadata discovery. No agent process is started here.
use super::*;
use crate::agent_console_view::NativeHistoryRequest;
use shelldeck_core::agent_remote_history::{
    history_command, parse_history, RemoteHistoryPage, RemoteHistoryQuery, MAX_REMOTE_HISTORY_BYTES,
};
use shelldeck_core::agent_remote_sessions::{
    discovery_page_command, parse_inventory, RemoteAgentSession, MAX_REMOTE_SESSION_INVENTORY_BYTES,
};

fn same_ssh_destination(
    a: &shelldeck_core::models::Connection,
    b: &shelldeck_core::models::Connection,
) -> bool {
    a.id == b.id
        && a.hostname == b.hostname
        && a.port == b.port
        && a.user == b.user
        && a.identity_file == b.identity_file
        && a.proxy_jump == b.proxy_jump
}

fn remote_read(
    connection: &shelldeck_core::models::Connection,
    command: &str,
    limit: usize,
) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| t!("agents.ssh_sessions.worker_failed").to_string())?;
    runtime.block_on(async {
        let session = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            shelldeck_ssh::client::SshClient::new().connect(connection),
        )
        .await
        .map_err(|_| t!("agents.ssh_sessions.timeout").to_string())?
        .map_err(|error| format!("{}: {error}", t!("agents.ssh_sessions.ssh_failed")))?;
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            session.exec_bounded(command, limit),
        )
        .await
        .map_err(|_| t!("agents.ssh_sessions.timeout").to_string())?
        .map_err(|error| format!("{}: {error}", t!("agents.ssh_sessions.read_failed")))?;
        if output.exit_code == Some(3) {
            return Err(t!("agents.ssh_sessions.folder_missing").to_string());
        }
        if !output.success() {
            return Err(t!("agents.ssh_sessions.read_failed").to_string());
        }
        String::from_utf8(output.stdout)
            .map_err(|_| t!("agents.ssh_sessions.read_failed").to_string())
    })
}

fn read_native_history(
    connection: &shelldeck_core::models::Connection,
    session: &RemoteAgentSession,
    query: &RemoteHistoryQuery,
    require_workdir: bool,
) -> Result<RemoteHistoryPage, String> {
    let history = history_command(session, query).map_err(|error| error.to_string())?;
    let command = if require_workdir {
        // Directory names are shell data. Reconnect verifies that execution can
        // still use the selected directory; passive history remains readable
        // even after that directory has been moved or removed.
        let cwd = shelldeck_core::util::shell_escape(&session.workdir);
        format!("test -d {cwd} || exit 3\n{history}")
    } else {
        history
    };
    let output = remote_read(connection, &command, MAX_REMOTE_HISTORY_BYTES)?;
    match parse_history(&output, session, query) {
        Ok(page) => Ok(page),
        Err(shelldeck_core::ShellDeckError::Config(message))
            if require_workdir
                && query.snapshot.is_some()
                && message
                    == "Saved history changed. Reload history to start a fresh snapshot." =>
        {
            // A rewritten transcript invalidates the passive cutoff, not the
            // native conversation. Explicit reconnect may revalidate its fresh
            // exact source once; the UI keeps the old cutoff and local history
            // separate when displaying this replacement snapshot. Ordinary
            // paging never falls back, and the unpinned retry cannot recurse.
            read_native_history(connection, session, &RemoteHistoryQuery::default(), true)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn imported_destination_matches(
    destinations: &std::collections::HashMap<
        (Uuid, String, String),
        shelldeck_core::models::Connection,
    >,
    connections: &[shelldeck_core::models::Connection],
    request: &shelldeck_core::agent_runtime::AgentRunRequest,
) -> bool {
    let shelldeck_core::agent_runtime::AgentTarget::Ssh { connection_id, .. } = &request.target
    else {
        return true;
    };
    let Some(native) = &request.resume_session else {
        return true;
    };
    destinations
        .get(&(
            *connection_id,
            request.provider.binary().into(),
            native.clone(),
        ))
        .is_none_or(|expected| {
            connections
                .iter()
                .any(|current| same_ssh_destination(current, expected))
        })
}

impl Workspace {
    pub(super) fn imported_agent_destination_matches(
        &self,
        request: &shelldeck_core::agent_runtime::AgentRunRequest,
    ) -> bool {
        imported_destination_matches(
            &self.imported_agent_destinations,
            &self.connections,
            request,
        )
    }

    pub(super) fn discover_ssh_agent_sessions(
        &mut self,
        connection_id: Uuid,
        request_id: Uuid,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|c| c.id == connection_id)
            .cloned()
        else {
            self.agent_console.update(cx, |view, cx| {
                view.set_ssh_sessions(
                    connection_id,
                    request_id,
                    Err(t!("agents.error.target_missing").to_string()),
                    cx,
                )
            });
            return;
        };
        let captured = connection.clone();
        let epoch = self.fleet_request_epoch;
        let task = cx.background_executor().spawn(async move {
            let command = discovery_page_command(cursor.as_deref()).map_err(|e| e.to_string())?;
            remote_read(&connection, &command, MAX_REMOTE_SESSION_INVENTORY_BYTES)
                .and_then(|text| parse_inventory(&text).map_err(|e| e.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fleet_request_epoch != epoch || !this.signed_in() {
                    return;
                }
                let result = if this
                    .connections
                    .iter()
                    .any(|c| same_ssh_destination(c, &captured))
                {
                    result
                } else {
                    Err(t!("agents.error.target_missing").to_string())
                };
                this.agent_console.update(cx, |view, cx| {
                    view.set_ssh_sessions(connection_id, request_id, result, cx)
                });
            });
        })
        .detach();
    }
    pub(super) fn reconnect_ssh_agent_session(
        &mut self,
        connection_id: Uuid,
        request_id: Uuid,
        session: RemoteAgentSession,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|c| c.id == connection_id)
            .cloned()
        else {
            self.agent_console.update(cx, |view, cx| {
                view.complete_ssh_reconnect(
                    request_id,
                    Err(t!("agents.error.target_missing").to_string()),
                    cx,
                )
            });
            return;
        };
        let captured = connection.clone();
        let epoch = self.fleet_request_epoch;
        let native_key = (
            connection_id,
            session.provider.binary().to_string(),
            session.session_id.clone(),
        );
        let query = self
            .agent_console
            .read(cx)
            .native_history_query(connection_id, &session);
        let task = cx
            .background_executor()
            .spawn(async move { read_native_history(&connection, &session, &query, true) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fleet_request_epoch != epoch || !this.signed_in() {
                    return;
                }
                let result = if this
                    .connections
                    .iter()
                    .any(|c| same_ssh_destination(c, &captured))
                {
                    result
                } else {
                    Err(t!("agents.error.target_missing").to_string())
                };
                if result.is_ok() {
                    this.imported_agent_destinations
                        .insert(native_key, captured);
                }
                this.agent_console.update(cx, |view, cx| {
                    view.complete_ssh_reconnect(request_id, result, cx)
                });
            });
        })
        .detach();
    }

    pub(super) fn load_ssh_agent_history(
        &mut self,
        request: NativeHistoryRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == request.connection_id)
            .cloned()
        else {
            self.agent_console.update(cx, |view, cx| {
                view.complete_native_history(
                    request,
                    Err(t!("agents.error.target_missing").to_string()),
                    cx,
                )
            });
            return;
        };
        let captured = connection.clone();
        let epoch = self.fleet_request_epoch;
        let remote = request.remote.clone();
        let query = request.query.clone();
        let task = cx
            .background_executor()
            .spawn(async move { read_native_history(&connection, &remote, &query, false) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fleet_request_epoch != epoch || !this.signed_in() {
                    return;
                }
                let result = if this
                    .connections
                    .iter()
                    .any(|current| same_ssh_destination(current, &captured))
                {
                    result
                } else {
                    Err(t!("agents.error.target_missing").to_string())
                };
                // This callback only supplies passive records. It cannot bind
                // provider resume state or acquire destination custody.
                this.agent_console.update(cx, |view, cx| {
                    view.complete_native_history(request, result, cx)
                });
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::imported_destination_matches;
    use shelldeck_core::{agent_runtime::*, models::Connection};
    // SDTEST-2812 / SDUC-562 — native identity must not cross a changed SSH destination.
    #[test]
    fn imported_resume_refuses_edited_or_removed_ssh_destination() {
        let original =
            Connection::new_manual("Server".into(), "first.invalid".into(), "user".into());
        let mut destinations = std::collections::HashMap::new();
        destinations.insert(
            (original.id, "codex".into(), "native-thread".into()),
            original.clone(),
        );
        let request = AgentRunRequest::new(
            AgentProvider::Codex,
            AgentTarget::Ssh {
                connection_id: original.id,
                connection_label: "Server".into(),
            },
            AgentAccessMode::ReadOnly,
            "/srv/repo",
            None,
            "continue",
        )
        .with_resume_session(Some("native-thread".into()));
        assert!(imported_destination_matches(
            &destinations,
            std::slice::from_ref(&original),
            &request
        ));
        let mut renamed = original.clone();
        renamed.alias = "Renamed".into();
        assert!(imported_destination_matches(
            &destinations,
            &[renamed],
            &request
        ));
        let mut changed = original.clone();
        changed.hostname = "second.invalid".into();
        assert!(!imported_destination_matches(
            &destinations,
            &[changed],
            &request
        ));
        let mut changed = original.clone();
        changed.user = "another-user".into();
        assert!(!imported_destination_matches(
            &destinations,
            &[changed],
            &request
        ));
        assert!(!imported_destination_matches(&destinations, &[], &request));
        assert!(imported_destination_matches(
            &destinations,
            &[],
            &request.with_resume_session(None)
        ));
    }
}
