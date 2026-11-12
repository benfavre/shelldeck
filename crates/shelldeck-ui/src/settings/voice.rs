//! Settings → IA → « Voix (ElevenLabs) »: the user's own ElevenLabs key for
//! speaking to agents.
//!
//! The section renders state and emits intents. Keychain access and every
//! ElevenLabs request run in `workspace/voice.rs` on the background executor;
//! the key never enters `AppConfig`, this view's state after saving, or logs.

use super::*;

/// What the section knows about the key. Set by Workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VoiceKeyStatus {
    /// Not read from the keychain yet.
    #[default]
    Unknown,
    Missing,
    /// In the keychain, not checked in this run.
    Saved,
    Checking,
    /// Checked against ElevenLabs: the account and its voices.
    Ready {
        tier: String,
        used: u64,
        limit: u64,
        voices: Vec<(String, String)>,
    },
    /// A stored key that ElevenLabs refused or could not be reached with.
    Failed(String),
    /// The system keychain could not be used: no key can be stored or read.
    Unavailable(String),
}

impl VoiceKeyStatus {
    /// A key is stored. A keychain error is not a key: nothing could use it.
    pub fn has_key(&self) -> bool {
        matches!(
            self,
            Self::Saved | Self::Checking | Self::Ready { .. } | Self::Failed(_)
        )
    }
}

impl SettingsView {
    /// Workspace reports the key's state (after a keychain read, a save, a
    /// removal or a check).
    pub fn set_voice_status(&mut self, status: VoiceKeyStatus, cx: &mut Context<Self>) {
        if let VoiceKeyStatus::Ready { voices, .. } = &status {
            self.voice_select = Some(build_voice_select(voices, &self.config, cx));
        }
        self.voice_status = status;
        cx.notify();
    }

    pub fn voice_status(&self) -> &VoiceKeyStatus {
        &self.voice_status
    }

    pub(super) fn render_voice_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let status = &self.voice_status;
        let (status_text, status_color) = match status {
            VoiceKeyStatus::Unknown | VoiceKeyStatus::Missing => (
                t!("settings.voice.status.missing").to_string(),
                ShellDeckColors::text_muted(),
            ),
            VoiceKeyStatus::Saved => (
                t!("settings.voice.status.saved").to_string(),
                ShellDeckColors::text_muted(),
            ),
            VoiceKeyStatus::Checking => (
                t!("settings.voice.status.checking").to_string(),
                ShellDeckColors::warning(),
            ),
            VoiceKeyStatus::Ready {
                tier, used, limit, ..
            } => (
                t!(
                    "settings.voice.status.ready",
                    tier = tier.as_str(),
                    used = used,
                    limit = limit
                )
                .to_string(),
                ShellDeckColors::success(),
            ),
            VoiceKeyStatus::Failed(error) | VoiceKeyStatus::Unavailable(error) => {
                (error.clone(), ShellDeckColors::error())
            }
        };
        let key_state = self.voice_key_state.clone();
        let has_key = status.has_key();
        let checking = matches!(status, VoiceKeyStatus::Checking);

        let mut key_buttons = div().flex().items_center().gap(px(8.0)).child(
            Button::new("voice-key-save", t!("settings.voice.key.save").to_string())
                .variant(ButtonVariant::Outline)
                .on_click(cx.listener(move |this, _, _window, cx| {
                    let value = key_state.read(cx).content().trim().to_string();
                    if value.is_empty() {
                        this.voice_status =
                            VoiceKeyStatus::Failed(t!("settings.voice.key.required").to_string());
                        cx.notify();
                        return;
                    }
                    key_state.update(cx, |state, cx| state.reset(cx));
                    this.voice_status = VoiceKeyStatus::Checking;
                    cx.emit(SettingsEvent::VoiceKeyStored(value));
                    cx.notify();
                })),
        );
        if has_key {
            key_buttons = key_buttons
                .child(
                    Button::new(
                        "voice-key-check",
                        t!("settings.voice.key.check").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .icon(IconSource::Named("refresh-cw".into()))
                    .disabled(checking)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.voice_status = VoiceKeyStatus::Checking;
                        cx.emit(SettingsEvent::VoiceCheckRequested);
                        cx.notify();
                    })),
                )
                .child(
                    Button::new(
                        "voice-key-delete",
                        t!("settings.voice.key.delete").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|_this, _, _window, cx| {
                        cx.emit(SettingsEvent::VoiceKeyDeleted);
                    })),
                );
        }

        let mut section = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pt(px(20.0))
                    .pb(px(4.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("settings.voice.title").to_string()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("settings.voice.intro").to_string()),
            )
            .child(Self::render_setting_row(
                t!("settings.voice.key.label").as_ref(),
                t!("settings.voice.key.description").as_ref(),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .w(px(300.0))
                    .child(
                        Input::new(&self.voice_key_state)
                            .w_full()
                            .size(InputSize::Sm)
                            .password(true)
                            .placeholder(if has_key {
                                t!("settings.voice.key.placeholder_saved").to_string()
                            } else {
                                t!("settings.voice.key.placeholder").to_string()
                            }),
                    )
                    .child(key_buttons)
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(status_color)
                            .child(status_text),
                    ),
            ));

        if let Some(select) = &self.voice_select {
            section = section.child(Self::render_setting_row(
                t!("settings.voice.voice.label").as_ref(),
                t!("settings.voice.voice.description").as_ref(),
                div().w(px(220.0)).child(select.clone()),
            ));
        }
        section
            .child(Self::render_setting_row(
                t!("settings.voice.dictation.label").as_ref(),
                t!("settings.voice.dictation.description").as_ref(),
                Self::bind_toggle(
                    "voice-dictation",
                    self.config.voice.dictation,
                    &entity,
                    |this, value| this.config.voice.dictation = value,
                ),
            ))
            .child(Self::render_setting_row(
                t!("settings.voice.read_replies.label").as_ref(),
                t!("settings.voice.read_replies.description").as_ref(),
                Self::bind_toggle(
                    "voice-read-replies",
                    self.config.voice.read_replies,
                    &entity,
                    |this, value| this.config.voice.read_replies = value,
                ),
            ))
    }
}

/// The account's voices; picking one stores its id and name in `[voice]`.
fn build_voice_select(
    voices: &[(String, String)],
    config: &AppConfig,
    cx: &mut Context<SettingsView>,
) -> Entity<Select<SharedString>> {
    let entries: Vec<(SharedString, SharedString)> = voices
        .iter()
        .map(|(id, name)| {
            (
                SharedString::from(id.clone()),
                SharedString::from(name.clone()),
            )
        })
        .collect();
    let current = config
        .voice
        .voice_id
        .clone()
        .or_else(|| voices.first().map(|(id, _)| id.clone()))
        .unwrap_or_default();
    let names: std::collections::HashMap<String, String> = voices.iter().cloned().collect();
    build_string_field_select(
        entries,
        &current,
        None,
        true,
        cx,
        |this| this.config.voice.voice_id.as_deref().unwrap_or(""),
        move |this, id| {
            this.config.voice.voice_name = names.get(&id).cloned();
            this.config.voice.voice_id = Some(id);
        },
    )
}

#[cfg(test)]
mod tests {
    use super::VoiceKeyStatus;
    use std::prelude::v1::test;

    // SDTEST-3084: every voice string resolves at its own path in both
    // locales (a key appended after a `[table]` header lands under that
    // table and shows raw in the app).
    #[test]
    fn sdtest_3084_voice_strings_resolve_in_both_locales() {
        let keys = [
            "settings.voice.title",
            "settings.voice.intro",
            "settings.voice.key.label",
            "settings.voice.key.save",
            "settings.voice.status.ready",
            "settings.voice.keychain_failed",
            "settings.voice.voice.label",
            "settings.voice.dictation.label",
            "settings.voice.read_replies.label",
            "agents.voice.dictate",
            "agents.voice.stop",
            "agents.voice.transcribing",
            "agents.voice.stop_reading",
            "agents.voice.no_key",
            "agents.voice.mic_failed",
            "agents.voice.read_failed",
            "toast.voice.key_ready",
            "toast.voice.key_deleted",
        ];
        for source in [
            include_str!("../../../shelldeck-core/locales/en.toml"),
            include_str!("../../../shelldeck-core/locales/fr.toml"),
        ] {
            let table: toml::Table = toml::from_str(source).expect("locale parses");
            for key in keys {
                let mut node = &toml::Value::Table(table.clone());
                for part in key.split('.') {
                    node = node
                        .get(part)
                        .unwrap_or_else(|| panic!("{key} missing at its own path"));
                }
                assert!(node.as_str().is_some_and(|text| !text.is_empty()), "{key}");
            }
        }
    }

    // SDTEST-3083: only a stored key turns voice on. A keychain error, no
    // key, or a status not read yet leave the microphone hidden; a key that
    // ElevenLabs refused is still a stored key (Settings offers to replace
    // it).
    #[test]
    fn sdtest_3083_only_a_stored_key_enables_voice() {
        for off in [
            VoiceKeyStatus::Unknown,
            VoiceKeyStatus::Missing,
            VoiceKeyStatus::Unavailable("keychain".into()),
        ] {
            assert!(!off.has_key(), "{off:?}");
        }
        for on in [
            VoiceKeyStatus::Saved,
            VoiceKeyStatus::Checking,
            VoiceKeyStatus::Failed("refused".into()),
            VoiceKeyStatus::Ready {
                tier: "creator".into(),
                used: 0,
                limit: 1,
                voices: Vec::new(),
            },
        ] {
            assert!(on.has_key(), "{on:?}");
        }
    }
}
