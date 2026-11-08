//! Inklura Chrome extension setup: download/refresh the package served by
//! Manage, open `chrome://extensions` with the folder path on the clipboard,
//! and install the optional Craft MCP bridge. All I/O runs on the
//! background executor; Settings → Général renders the resulting state.

use super::*;
use crate::settings::InkluraExtensionPanel;
use shelldeck_core::config::inklura_extension;

impl Workspace {
    fn set_inklura_extension_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.settings.update(cx, |settings, cx| {
            let mut panel = settings.inklura_extension().clone();
            panel.busy = busy;
            settings.set_inklura_extension(panel, cx);
        });
    }

    fn account_email(&self) -> Option<String> {
        self.app_config
            .account
            .as_ref()
            .map(|a| a.email.clone())
            .filter(|e| !e.is_empty())
    }

    /// Re-read install + Chrome profile state (e.g. after the user loaded
    /// the extension in Chrome and comes back to Settings).
    pub(super) fn refresh_inklura_extension_panel(&mut self, cx: &mut Context<Self>) {
        let panel = InkluraExtensionPanel::detect(self.account_email().as_deref());
        self.settings
            .update(cx, |settings, cx| settings.set_inklura_extension(panel, cx));
    }

    /// Palette entry point: show the Settings section, then run the setup so
    /// the instructions stay visible after the toast.
    pub(super) fn setup_inklura_extension(&mut self, cx: &mut Context<Self>) {
        self.settings.update(cx, |settings, cx| {
            settings.set_active_tab(crate::settings::SettingsTab::General, cx);
        });
        self.open_settings(cx);
        self.install_inklura_extension(cx);
    }

    /// Download (or refresh in place) the extension, copy the folder path
    /// and open `chrome://extensions` for the « Load unpacked » step.
    pub(super) fn install_inklura_extension(&mut self, cx: &mut Context<Self>) {
        if self.settings.read(cx).inklura_extension().busy {
            return;
        }
        self.set_inklura_extension_busy(true, cx);
        let base_url = self.app_config.cloud_sync.base_url.clone();
        let email = self.account_email();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    inklura_extension::install(&base_url).map(|ext| {
                        let opened = inklura_extension::open_extensions_page(email.as_deref());
                        (ext, opened)
                    })
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.refresh_inklura_extension_panel(cx);
                match result {
                    Ok((ext, opened)) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            ext.extension_dir.display().to_string(),
                        ));
                        let message = match opened {
                            Ok(profile) => t!(
                                "settings.inklura_extension.toast.installed",
                                version = ext.version.clone(),
                                profile = profile_label(profile.as_ref())
                            ),
                            Err(_) => t!(
                                "settings.inklura_extension.toast.installed_no_chrome",
                                version = ext.version.clone()
                            ),
                        };
                        ws.show_toast(message.to_string(), ToastLevel::Success, cx);
                    }
                    Err(e) => ws.show_toast(
                        t!(
                            "settings.inklura_extension.toast.failed",
                            error = e.to_string()
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    pub(super) fn open_inklura_extension_page(&mut self, cx: &mut Context<Self>) {
        let email = self.account_email();
        match inklura_extension::open_extensions_page(email.as_deref()) {
            Err(e) => self.show_toast(
                t!(
                    "settings.inklura_extension.toast.chrome_failed",
                    error = e.to_string()
                )
                .to_string(),
                ToastLevel::Error,
                cx,
            ),
            Ok(profile) => self.show_toast(
                t!(
                    "settings.inklura_extension.toast.path_copied",
                    profile = profile_label(profile.as_ref())
                )
                .to_string(),
                ToastLevel::Info,
                cx,
            ),
        }
    }

    pub(super) fn install_inklura_bridge(
        &mut self,
        workspace: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(ext) = self.settings.read(cx).inklura_extension().installed.clone() else {
            return;
        };
        self.set_inklura_extension_busy(true, cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { inklura_extension::install_native_bridge(&ext, &workspace) })
                .await;
            let _ = this.update(cx, |ws, cx| {
                ws.set_inklura_extension_busy(false, cx);
                match result {
                    Ok(report) => ws.show_toast(
                        t!(
                            "settings.inklura_extension.toast.bridge_installed",
                            report = report
                        )
                        .to_string(),
                        ToastLevel::Success,
                        cx,
                    ),
                    Err(e) => ws.show_toast(
                        t!(
                            "settings.inklura_extension.toast.bridge_failed",
                            error = e.to_string()
                        )
                        .to_string(),
                        ToastLevel::Error,
                        cx,
                    ),
                }
            });
        })
        .detach();
    }
}

/// « Personne 1 (ben@…) » for toasts; Chrome's default window otherwise.
fn profile_label(profile: Option<&inklura_extension::ChromeProfile>) -> String {
    match profile {
        Some(p) if !p.email.is_empty() => format!("{} ({})", p.name, p.email),
        Some(p) => p.name.clone(),
        None => "Chrome".to_string(),
    }
}
