//! `general.session_daemon`: the daemon files of this machine
//! (`crate::sessiond_files`). With the setting on and no daemon installed,
//! the workspace fetches it in the background and says what happened,
//! instead of leaving the setting on with no effect.

use super::Workspace;
use crate::sessiond_files::{self, fetch_failure_text, Status};
use crate::t;
use crate::toast::ToastLevel;
use gpui::{AsyncApp, Context};
use shelldeck_update::sessiond::FetchErrorKind;

impl Workspace {
    /// Make sure the daemon of this machine is installed when the setting
    /// is on. `announce`: the user just turned the setting on, so every
    /// outcome is said; at startup only a result worth a message is.
    ///
    /// Returns at once. Terminals opened before the daemon is there are
    /// ordinary ones; the ones opened after it use it.
    pub(super) fn ensure_session_daemon_files(&mut self, announce: bool, cx: &mut Context<Self>) {
        if !self.app_config.general.session_daemon {
            return;
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // Off the UI thread: a few `stat`, and the cleanup of files an
            // update has replaced.
            let (status, claimed) = cx
                .background_executor()
                .spawn(async {
                    sessiond_files::remove_superseded();
                    let status = sessiond_files::status();
                    let claimed = matches!(status, Status::Missing | Status::FetchFailed(_))
                        && sessiond_files::begin_fetch();
                    (status, claimed)
                })
                .await;
            if !claimed {
                if announce && status == Status::Unavailable {
                    let _ = this.update(cx, |workspace, cx| {
                        workspace.show_toast(
                            t!("toast.session_daemon_files.unavailable").to_string(),
                            ToastLevel::Warning,
                            cx,
                        );
                    });
                }
                return;
            }
            if announce {
                let _ = this.update(cx, |workspace, cx| {
                    workspace.show_toast(
                        t!("toast.session_daemon_files.downloading").to_string(),
                        ToastLevel::Info,
                        cx,
                    );
                });
            }
            let result = cx
                .background_executor()
                .spawn(async { sessiond_files::fetch() })
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(()) => workspace.show_toast(
                    t!("toast.session_daemon_files.installed").to_string(),
                    ToastLevel::Success,
                    cx,
                ),
                // The update that is on its way brings the files: nothing
                // to report at startup.
                Err(FetchErrorKind::OtherVersion(_)) if !announce => {}
                Err(kind) => workspace.show_toast(
                    t!(
                        "toast.session_daemon_files.failed",
                        reason = fetch_failure_text(&kind).as_str()
                    )
                    .to_string(),
                    ToastLevel::Warning,
                    cx,
                ),
            });
        })
        .detach();
    }
}
