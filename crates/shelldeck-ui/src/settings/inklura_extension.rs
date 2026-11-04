//! Settings → Général → « Extension Chrome Inklura ».
//!
//! The section only renders state and emits intents; downloads, launching
//! Chrome and the Craft bridge installer run in
//! `workspace/inklura_extension.rs` on the background executor.

use super::*;
use shelldeck_core::config::inklura_extension::{self, InstalledExtension};

/// What the section shows. Owned by `SettingsView`, refreshed by Workspace
/// after each operation (`set_inklura_extension`).
#[derive(Debug, Clone)]
pub struct InkluraExtensionPanel {
    pub installed: Option<InstalledExtension>,
    pub busy: bool,
    pub chrome_available: bool,
}

impl InkluraExtensionPanel {
    /// Read the on-disk state (a manifest read + PATH lookup; cheap).
    pub fn detect() -> Self {
        Self {
            installed: inklura_extension::installed(),
            busy: false,
            chrome_available: inklura_extension::chrome_available(),
        }
    }
}

impl SettingsView {
    pub fn inklura_extension(&self) -> &InkluraExtensionPanel {
        &self.inklura_extension
    }

    pub fn set_inklura_extension(&mut self, panel: InkluraExtensionPanel, cx: &mut Context<Self>) {
        self.inklura_extension = panel;
        cx.notify();
    }

    fn copy_inklura_extension_path(&self, cx: &mut Context<Self>) {
        if let Some(ext) = &self.inklura_extension.installed {
            cx.write_to_clipboard(ClipboardItem::new_string(
                ext.extension_dir.display().to_string(),
            ));
        }
    }

    /// Pick the folder the Craft apps may read/write, then ask Workspace to
    /// run the package's bridge installer with the computed extension ID.
    fn pick_craft_workspace(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                t!("settings.inklura_extension.bridge.prompt")
                    .to_string()
                    .into(),
            ),
            starting_directory: shelldeck_core::util::home_dir(),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(workspace) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |_, cx| {
                cx.emit(SettingsEvent::InkluraExtensionBridgeRequested(workspace));
            });
        })
        .detach();
    }

    pub(super) fn render_inklura_extension_settings(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let panel = &self.inklura_extension;
        let value_text = |s: String| {
            div()
                .min_w(px(0.0))
                .max_w(px(320.0))
                .truncate()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ShellDeckColors::text_primary())
                .child(s)
        };

        let status = match &panel.installed {
            Some(ext) => t!(
                "settings.inklura_extension.status.installed",
                version = ext.version.clone()
            )
            .to_string(),
            None => t!("settings.inklura_extension.status.missing").to_string(),
        };
        let install_label = match (panel.busy, panel.installed.is_some()) {
            (true, _) => t!("settings.inklura_extension.install.busy"),
            (false, true) => t!("settings.inklura_extension.install.update"),
            (false, false) => t!("settings.inklura_extension.install.button"),
        };

        let mut section = div()
            .flex()
            .flex_col()
            .child(Self::render_about_section(
                t!("settings.inklura_extension.section").as_ref(),
            ))
            .child(Self::render_setting_row(
                t!("settings.inklura_extension.status.label").as_ref(),
                t!("settings.inklura_extension.status.description").as_ref(),
                value_text(status),
            ));

        if let Some(ext) = &panel.installed {
            section = section
                .child(Self::render_setting_row(
                    t!("settings.inklura_extension.folder.label").as_ref(),
                    t!("settings.inklura_extension.folder.description").as_ref(),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w(px(0.0))
                        .child(value_text(ext.extension_dir.display().to_string()))
                        .child(
                            Button::new(
                                "inklura-extension-copy-path",
                                t!("settings.inklura_extension.folder.copy").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .icon(IconSource::Named("copy".into()))
                            .on_click(cx.listener(
                                |this, _, _window, cx| {
                                    this.copy_inklura_extension_path(cx);
                                },
                            )),
                        ),
                ))
                .child(Self::render_setting_row(
                    t!("settings.inklura_extension.id.label").as_ref(),
                    t!("settings.inklura_extension.id.description").as_ref(),
                    value_text(ext.extension_id.clone()),
                ));
        }

        let mut actions = div()
            .mt(px(12.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(
                Button::new("inklura-extension-install", install_label.to_string())
                    .variant(ButtonVariant::Default)
                    .icon(IconSource::Named("download".into()))
                    .disabled(panel.busy)
                    .on_click(cx.listener(|_this, _, _window, cx| {
                        cx.emit(SettingsEvent::InkluraExtensionInstallRequested);
                    })),
            );
        if panel.installed.is_some() {
            actions = actions.child(
                Button::new(
                    "inklura-extension-open-chrome",
                    t!("settings.inklura_extension.open_chrome").to_string(),
                )
                .variant(ButtonVariant::Outline)
                .icon(IconSource::Named("external-link".into()))
                .disabled(panel.busy || !panel.chrome_available)
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.copy_inklura_extension_path(cx);
                    cx.emit(SettingsEvent::InkluraExtensionOpenChromeRequested);
                })),
            );
            if inklura_extension::native_bridge_supported() {
                actions = actions.child(
                    Button::new(
                        "inklura-extension-bridge",
                        t!("settings.inklura_extension.bridge.button").to_string(),
                    )
                    .variant(ButtonVariant::Outline)
                    .icon(IconSource::Named("folder".into()))
                    .disabled(panel.busy)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.pick_craft_workspace(cx);
                    })),
                );
            }
        }
        section = section.child(actions);

        // The one step Chrome reserves for the user, spelled out so the
        // instructions survive after the toast is gone.
        let mut steps = div()
            .mt(px(12.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_muted());
        let step_keys: &[&str] = if panel.chrome_available {
            &[
                "settings.inklura_extension.steps.one",
                "settings.inklura_extension.steps.two",
                "settings.inklura_extension.steps.three",
                "settings.inklura_extension.steps.four",
            ]
        } else {
            &["settings.inklura_extension.chrome_missing"]
        };
        for key in step_keys {
            steps = steps.child(div().max_w(px(560.0)).child(t!(*key).to_string()));
        }
        section.child(steps)
    }
}
