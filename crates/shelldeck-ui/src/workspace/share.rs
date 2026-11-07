use super::*;

impl Workspace {
    pub(super) fn open_personal_share(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            return;
        }
        let manage = self.account_base_url();
        let token = self.app_config.cloud_sync.token.clone();
        self.user_home_tab = UserHomeTab::Share;
        self.personal_share.update(cx, |view, cx| {
            view.configure(manage, token, cx);
            view.refresh(cx);
        });
        cx.notify();
    }

    /// Files handed over by the OS. Signed-out launches keep them until the
    /// login completes (an account change otherwise clears Share state).
    pub fn stage_external_share(&mut self, files: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        self.show_main_presentation(cx);
        if !self.signed_in() {
            self.pending_share_files = files;
            self.show_login_form(cx);
            self.show_toast(
                t!("share.external.sign_in").to_string(),
                ToastLevel::Info,
                cx,
            );
            return;
        }
        if self.effective_mode() != AppMode::User && self.can_access_mode(AppMode::User) {
            self.set_mode(AppMode::User, cx);
        }
        self.open_personal_share(cx);
        self.personal_share
            .update(cx, |view, cx| view.stage_external(files, cx));
        cx.notify();
    }

    /// Re-applies a share intent that arrived before sign-in.
    pub(super) fn resume_pending_share(&mut self, cx: &mut Context<Self>) {
        if self.pending_share_files.is_empty() || !self.signed_in() {
            return;
        }
        let files = std::mem::take(&mut self.pending_share_files);
        self.stage_external_share(files, cx);
    }
}
