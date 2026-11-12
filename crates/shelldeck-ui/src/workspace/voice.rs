//! The user's own ElevenLabs key: keychain storage, checks, and keeping the
//! Settings section and the agent console in step. All keychain and network
//! work runs on the background executor; the key is never logged, stored in
//! `AppConfig`, or kept in a view.

use super::*;
use crate::settings::VoiceKeyStatus;
use shelldeck_core::config::elevenlabs::{ElevenLabsClient, ELEVENLABS_KEY_ID};
use shelldeck_core::config::keychain;

impl Workspace {
    /// Read whether a key is stored (startup, sign-in) and tell the views.
    pub(super) fn load_voice_key_status(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let stored = cx
                .background_executor()
                .spawn(async move {
                    keychain::get_ai_api_key(ELEVENLABS_KEY_ID)
                        .map(|key| key.is_some_and(|key| !key.trim().is_empty()))
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let status = match stored {
                    Ok(true) => VoiceKeyStatus::Saved,
                    Ok(false) => VoiceKeyStatus::Missing,
                    Err(error) => VoiceKeyStatus::Unavailable(
                        t!("settings.voice.keychain_failed", error = error.to_string()).to_string(),
                    ),
                };
                ws.apply_voice_status(status, cx);
            });
        })
        .detach();
    }

    /// Settings showed a new key: store it, then check it.
    pub(super) fn store_voice_key(&mut self, key: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let stored = cx
                .background_executor()
                .spawn(async move { keychain::store_ai_api_key(ELEVENLABS_KEY_ID, key.trim()) })
                .await;
            let _ = this.update(cx, |ws, cx| match stored {
                Ok(()) => {
                    ws.apply_voice_status(VoiceKeyStatus::Checking, cx);
                    ws.check_voice_key(cx);
                }
                Err(error) => ws.apply_voice_status(
                    VoiceKeyStatus::Unavailable(
                        t!("settings.voice.keychain_failed", error = error.to_string()).to_string(),
                    ),
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(super) fn delete_voice_key(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let deleted = cx
                .background_executor()
                .spawn(async move { keychain::delete_ai_api_key(ELEVENLABS_KEY_ID) })
                .await;
            let _ = this.update(cx, |ws, cx| match deleted {
                Ok(()) => {
                    ws.apply_voice_status(VoiceKeyStatus::Missing, cx);
                    ws.show_toast(
                        t!("toast.voice.key_deleted").to_string(),
                        ToastLevel::Info,
                        cx,
                    );
                }
                Err(error) => ws.apply_voice_status(
                    VoiceKeyStatus::Unavailable(
                        t!("settings.voice.keychain_failed", error = error.to_string()).to_string(),
                    ),
                    cx,
                ),
            });
        })
        .detach();
    }

    /// Ask ElevenLabs who the key belongs to and which voices it has.
    pub(super) fn check_voice_key(&mut self, cx: &mut Context<Self>) {
        let base = self.app_config.voice.base_url.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let checked = cx
                .background_executor()
                .spawn(async move {
                    let key = keychain::get_ai_api_key(ELEVENLABS_KEY_ID)
                        .map_err(|error| error.to_string())?
                        .filter(|key| !key.trim().is_empty())
                        .ok_or_else(|| t!("agents.voice.no_key").to_string())?;
                    let client =
                        ElevenLabsClient::new(&base, &key).map_err(|error| error.to_string())?;
                    let account = client.account().map_err(|error| error.to_string())?;
                    let voices = client
                        .voices()
                        .map_err(|error| error.to_string())?
                        .into_iter()
                        .map(|voice| (voice.voice_id, voice.name))
                        .collect::<Vec<_>>();
                    Ok::<_, String>(VoiceKeyStatus::Ready {
                        tier: account.tier,
                        used: account.character_count,
                        limit: account.character_limit,
                        voices,
                    })
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let ready = checked.is_ok();
                ws.apply_voice_status(checked.unwrap_or_else(VoiceKeyStatus::Failed), cx);
                if ready {
                    ws.show_toast(
                        t!("toast.voice.key_ready").to_string(),
                        ToastLevel::Success,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// One place updates both views.
    fn apply_voice_status(&mut self, status: VoiceKeyStatus, cx: &mut Context<Self>) {
        let has_key = status.has_key();
        self.settings.update(cx, |settings, cx| {
            settings.set_voice_status(status, cx);
        });
        self.push_voice_to_agents(has_key, cx);
    }

    /// The agent console needs to know whether a key exists and the choices.
    pub(super) fn push_voice_to_agents(&mut self, has_key: bool, cx: &mut Context<Self>) {
        let config = self.app_config.voice.clone();
        self.agent_console.update(cx, |console, cx| {
            console.set_voice(has_key, config, cx);
        });
    }

    /// Whether Settings currently knows of a stored key.
    pub(super) fn voice_key_known(&self, cx: &App) -> bool {
        self.settings.read(cx).voice_status().has_key()
    }
}
