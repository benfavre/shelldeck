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
}
