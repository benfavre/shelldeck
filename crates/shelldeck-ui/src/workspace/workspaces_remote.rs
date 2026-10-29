//! Folder-picker admission and draft custody. Browsing never registers a checkout.
use super::remote_picker::{RemoteFolderPicker, RemoteFolderPickerEvent};
use super::*;

pub(super) struct RemoteFolderSelection {
    pub(super) view: Entity<RemoteFolderPicker>,
    connection: Connection,
    input: String,
    _subscription: Subscription,
}

fn same_connection(a: &Connection, b: &Connection) -> bool {
    native_lifecycle::same_ssh_authority(a, b)
}

impl WorkspaceHubView {
    pub(super) fn request_remote_folder_picker(&mut self, cx: &mut Context<Self>) {
        if !self.onboarding_open || !self.onboarding_ssh {
            return;
        }
        let Some(connection_id) = self
            .onboarding_connection
            .filter(|id| self.connections.contains_key(id))
        else {
            return;
        };
        self.cancel_remote_folder_picker(cx);
        let input = self.onboarding_root.read(cx).content().to_owned();
        cx.emit(WorkspaceHubEvent::BrowseRemoteFolder {
            request: self.remote_browse_epoch,
            connection_id,
            input,
        });
    }

    pub(in crate::workspace) fn cancel_remote_folder_picker(&mut self, cx: &mut Context<Self>) {
        self.remote_browse_epoch = self.remote_browse_epoch.wrapping_add(1);
        if let Some(picker) = self.remote_folder_picker.take() {
            picker.view.update(cx, |picker, cx| picker.close(cx));
            cx.notify();
        }
    }

    pub(in crate::workspace) fn open_remote_folder_picker(
        &mut self,
        request: u64,
        connection: Connection,
        input: String,
        cx: &mut Context<Self>,
    ) {
        if request != self.remote_browse_epoch
            || !self.onboarding_open
            || !self.onboarding_ssh
            || self.onboarding_connection != Some(connection.id)
            || self.onboarding_root.read(cx).content() != input
        {
            return;
        }
        let initial = (!input.is_empty() && input != "~").then(|| input.clone());
        let view = cx.new(|cx| RemoteFolderPicker::new(connection.clone(), initial, cx));
        let subscription = cx.subscribe(
            &view,
            move |hub, picker, event: &RemoteFolderPickerEvent, cx| {
                let Some(current) = hub.remote_folder_picker.as_ref() else {
                    return;
                };
                if current.view.entity_id() != picker.entity_id()
                    || request != hub.remote_browse_epoch
                    || !hub.onboarding_open
                    || !hub.onboarding_ssh
                    || hub.onboarding_connection != Some(current.connection.id)
                    || hub.onboarding_root.read(cx).content() != current.input
                {
                    return;
                }
                if let RemoteFolderPickerEvent::Selected(path) = event {
                    if RemotePosixPath::new(path.clone()).is_err() {
                        return;
                    }
                    hub.onboarding_browse_authority = Some(current.connection.clone());
                    hub.onboarding_root
                        .update(cx, |state, cx| state.replace_content(path.clone(), cx));
                    hub.error = None;
                }
                hub.cancel_remote_folder_picker(cx);
                cx.notify();
            },
        );
        self.remote_folder_picker = Some(RemoteFolderSelection {
            view,
            connection,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(in crate::workspace) fn refresh_browse_connections(
        &mut self,
        connections: &[Connection],
        cx: &mut Context<Self>,
    ) {
        let retired = self.executor.update_connections(connections);
        let cancel: Vec<_> = self
            .pending_requests
            .iter()
            .filter_map(|(workspace, request)| {
                let AuthorizedLaunchHost::Ssh { connection_id, .. } = &request.host else {
                    return None;
                };
                (retired.contains(connection_id)
                    && matches!(
                        self.creation.state(*workspace),
                        Some(BackgroundWorkspaceCreateState::Running { .. })
                    ))
                .then_some(*workspace)
            })
            .collect();
        // Fence completion synchronously in the reducer before the background
        // cancellation handshake. An old receipt must never attach to a new host.
        for workspace in cancel {
            self.request_cancel(workspace, cx);
        }
        if self.remote_folder_picker.as_ref().is_some_and(|current| {
            !connections
                .iter()
                .any(|connection| same_connection(connection, &current.connection))
        }) {
            self.cancel_remote_folder_picker(cx);
            self.onboarding_root
                .update(cx, |input, cx| input.replace_content(String::new(), cx));
        }
        if self
            .onboarding_browse_authority
            .as_ref()
            .is_some_and(|picked| {
                !connections
                    .iter()
                    .any(|connection| same_connection(connection, picked))
            })
        {
            self.onboarding_browse_authority = None;
            self.onboarding_root
                .update(cx, |input, cx| input.replace_content(String::new(), cx));
        }
        let labels: HashMap<_, _> = connections
            .iter()
            .map(|connection| (connection.id, connection.display_name().to_owned()))
            .collect();
        if self.connections != labels {
            self.connections = labels;
            self.onboarding_connection_select = None;
            if self
                .onboarding_connection
                .is_some_and(|id| !self.connections.contains_key(&id))
            {
                self.onboarding_connection = None;
                self.onboarding_root
                    .update(cx, |input, cx| input.replace_content(String::new(), cx));
            }
            self.refresh_checkout_select(self.launcher.checkout, cx);
            cx.notify();
        }
    }
}
