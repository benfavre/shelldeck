use super::*;

impl Workspace {
    pub(super) fn browse_remote_project(
        &mut self,
        request: u64,
        connection_id: Uuid,
        input: String,
        cx: &mut Context<Self>,
    ) {
        if !self.can_access_mode(AppMode::Dev)
            || self.effective_mode() != AppMode::Dev
            || self.settings_open
            || self.active_view != ActiveView::Workspaces
        {
            return;
        }
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .cloned()
        else {
            return;
        };
        self.workspace_hub.update(cx, |hub, cx| {
            hub.open_remote_folder_picker(request, connection, input, cx)
        });
    }
}
