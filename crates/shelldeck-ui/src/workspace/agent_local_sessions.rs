//! Desktop native CLI files use the same passive history and explicit reconnect UI as SSH.
use super::*;
use crate::agent_console_view::NativeHistoryRequest;
use shelldeck_core::agent_local_sessions::{discover, read_history, LocalSessionRoots};
use shelldeck_core::agent_remote_history::RemoteHistoryQuery;
use shelldeck_core::agent_remote_sessions::RemoteAgentSession;

impl Workspace {
    pub(super) fn discover_local_agent_sessions(
        &mut self,
        request_id: Uuid,
        cursor: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let epoch = self.fleet_request_epoch;
        let task = cx.background_executor().spawn(async move {
            let roots = LocalSessionRoots::from_environment().map_err(|e| e.to_string())?;
            discover(&roots, cursor.as_deref()).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fleet_request_epoch != epoch || !this.signed_in() {
                    return;
                }
                this.agent_console.update(cx, |view, cx| {
                    view.set_native_sessions(None, request_id, result, cx)
                });
            });
        })
        .detach();
    }
    pub(super) fn reconnect_local_agent_session(
        &mut self,
        request_id: Uuid,
        session: RemoteAgentSession,
        cx: &mut Context<Self>,
    ) {
        let epoch = self.fleet_request_epoch;
        let query = self.agent_console.read(cx).local_history_query(&session);
        let task = cx.background_executor().spawn(async move {
            let roots = LocalSessionRoots::from_environment().map_err(|e| e.to_string())?;
            match read_history(&roots, &session, &query, true) {
                Err(shelldeck_core::ShellDeckError::Config(message)) if query.snapshot.is_some() && message == "Saved history changed. Reload history to start a fresh snapshot." => read_history(&roots, &session, &RemoteHistoryQuery::default(), true).map_err(|e| e.to_string()),
                result => result.map_err(|e| e.to_string()),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fleet_request_epoch != epoch || !this.signed_in() {
                    return;
                }
                this.agent_console.update(cx, |view, cx| {
                    view.complete_ssh_reconnect(request_id, result, cx)
                });
            });
        })
        .detach();
    }
    pub(super) fn load_local_agent_history(
        &mut self,
        request: NativeHistoryRequest,
        cx: &mut Context<Self>,
    ) {
        let epoch = self.fleet_request_epoch;
        let remote = request.remote.clone();
        let query = request.query.clone();
        let task = cx.background_executor().spawn(async move {
            let roots = LocalSessionRoots::from_environment().map_err(|e| e.to_string())?;
            read_history(&roots, &remote, &query, false).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.fleet_request_epoch != epoch || !this.signed_in() {
                    return;
                }
                this.agent_console.update(cx, |view, cx| {
                    view.complete_native_history(request, result, cx)
                });
            });
        })
        .detach();
    }
}
