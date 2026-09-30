use super::*;
use shelldeck_core::ShellDeckError;

/// Independent reads may run together; only the newest read of each surface
/// can publish. Session changes invalidate every outstanding read or write.
#[derive(Clone, Copy)]
pub(super) enum ManageRead {
    Login,
    Account,
    Sites,
    Issues,
    IssueDetail,
    Support,
    SupportDetail,
    Sync,
    People,
}

#[derive(Clone, Copy)]
pub(super) struct ManageRequest {
    session: u64,
    read: Option<(ManageRead, u64)>,
}

#[derive(Default)]
pub(super) struct ManageRequests {
    session: u64,
    reads: [u64; 9],
}

#[derive(Debug, PartialEq, Eq)]
enum ManageDisposition {
    Apply,
    Ignore,
    Expired,
}

impl ManageRequests {
    pub(super) fn session(&self) -> ManageRequest {
        ManageRequest {
            session: self.session,
            read: None,
        }
    }

    pub(super) fn current(&self, read: ManageRead) -> ManageRequest {
        ManageRequest {
            session: self.session,
            read: Some((read, self.reads[read as usize])),
        }
    }

    pub(super) fn begin(&mut self, read: ManageRead) -> ManageRequest {
        let serial = &mut self.reads[read as usize];
        *serial = serial.wrapping_add(1);
        self.current(read)
    }

    pub(super) fn is_current(&self, request: ManageRequest) -> bool {
        request.session == self.session
            && request
                .read
                .is_none_or(|(read, serial)| self.reads[read as usize] == serial)
    }

    pub(super) fn change_session(&mut self) {
        self.session = self.session.wrapping_add(1);
    }

    fn disposition(
        &self,
        request: ManageRequest,
        error: Option<&ShellDeckError>,
    ) -> ManageDisposition {
        if !self.is_current(request) {
            return ManageDisposition::Ignore;
        }
        if error.is_some_and(|error| {
            cloud_account::classify_api_error(error) == cloud_account::ApiFailure::AuthRejected
        }) {
            ManageDisposition::Expired
        } else {
            ManageDisposition::Apply
        }
    }
}

impl Workspace {
    /// A 403 is a permission failure, not a revoked login. Ignore errors from
    /// old sessions before considering 401, so an old token cannot log out a
    /// newly signed-in account or produce repeated expiry notifications.
    pub(super) fn accept_manage_result(
        &mut self,
        request: ManageRequest,
        error: Option<&ShellDeckError>,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.manage_requests.disposition(request, error) {
            ManageDisposition::Apply => true,
            ManageDisposition::Ignore => false,
            ManageDisposition::Expired => {
                self.invalidate_cloud_session(cx);
                self.account_status = AccountStatus::Rejected;
                self.show_toast(
                    t!("toast.session.expired").to_string(),
                    ToastLevel::Warning,
                    cx,
                );
                false
            }
        }
    }

    /// Fetching happens in the background. Commit only after checking the
    /// session on the UI thread, where logout and connection edits are ordered.
    pub(super) fn apply_cloud_profiles(
        &mut self,
        payload: shelldeck_core::config::cloud_sync::SyncPayload,
        cx: &mut Context<Self>,
    ) -> shelldeck_core::Result<shelldeck_core::config::cloud_sync::MergeStats> {
        let mut store = ConnectionStore::load()?;
        let stats =
            shelldeck_core::config::cloud_sync::merge_profiles(&mut store, &payload.connections);
        if stats.changed() {
            store.save()?;
        }
        self.reload_connections_after_sync(cx);
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::{ManageDisposition, ManageRead, ManageRequests};
    use shelldeck_core::ShellDeckError;

    // SDTEST-1937 — the later selected request must survive out-of-order reads,
    // while a list refresh must not accidentally cancel its detail request.
    #[test]
    fn newer_detail_survives_delayed_response_and_independent_list_refresh() {
        let mut requests = ManageRequests::default();
        let old = requests.begin(ManageRead::IssueDetail);
        let old_write_detail = requests.current(ManageRead::IssueDetail);
        let write_session = requests.session();
        let new = requests.begin(ManageRead::IssueDetail);
        let list = requests.begin(ManageRead::Issues);
        let mut displayed = None;
        for (request, id) in [(new, "selected"), (old, "previous")] {
            if requests.is_current(request) {
                displayed = Some(id);
            }
        }
        assert_eq!(displayed, Some("selected"));
        assert!(requests.is_current(write_session));
        assert!(!requests.is_current(old_write_detail));
        assert!(requests.is_current(list));
        let obsolete_list = list;
        requests.begin(ManageRead::Issues);
        assert!(!requests.is_current(obsolete_list));
        assert!(requests.is_current(new));
    }

    // SDTEST-1938 — old reads/writes must not restore data or reject a new
    // account, including a re-login that uses the same bearer token.
    #[test]
    fn logout_and_relogin_retire_all_previous_reads_and_writes() {
        let mut requests = ManageRequests::default();
        let old_read = requests.begin(ManageRead::Account);
        let old_write = requests.session();
        requests.change_session();
        let logged_out = requests.session();
        requests.change_session();
        let new_read = requests.begin(ManageRead::Account);
        let new_write = requests.session();
        let mut accepted = Vec::new();
        for (request, label) in [
            (old_read, "old identity"),
            (old_write, "old write"),
            (logged_out, "logged out"),
            (new_read, "new identity"),
            (new_write, "new write"),
        ] {
            if requests.is_current(request) {
                accepted.push(label);
            }
        }
        assert_eq!(accepted, ["new identity", "new write"]);
    }

    // SDTEST-1939 — a permission error stays local; the first current 401
    // ends the session, making concurrent rejections and old successes inert.
    #[test]
    fn permission_failures_keep_session_but_current_revocation_expires_once() {
        let mut requests = ManageRequests::default();
        let request = requests.begin(ManageRead::Support);
        let concurrent = requests.begin(ManageRead::Issues);
        let forbidden = ShellDeckError::Connection("staff only (403)".into());
        let rejected = ShellDeckError::Connection("session token rejected (401)".into());
        assert_eq!(
            requests.disposition(request, Some(&forbidden)),
            ManageDisposition::Apply
        );
        assert_eq!(
            requests.disposition(request, Some(&rejected)),
            ManageDisposition::Expired
        );
        requests.change_session();
        assert_eq!(
            requests.disposition(concurrent, Some(&rejected)),
            ManageDisposition::Ignore
        );
        assert_eq!(
            requests.disposition(request, None),
            ManageDisposition::Ignore
        );
        let new = requests.begin(ManageRead::Support);
        assert_eq!(requests.disposition(new, None), ManageDisposition::Apply);
    }
}
