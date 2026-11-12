//! Speaking to agents with the user's own ElevenLabs key: push-to-talk
//! dictation into the composer, and the final reply of a run read aloud.
//!
//! The key is read from the OS keychain on the background executor for each
//! request and never kept in the view. Settings owns the key and the
//! `[voice]` choices; Workspace pushes them here with `set_voice`.

use super::*;
use crate::voice::{Recorder, Speaker, STT_SAMPLE_RATE};
use shelldeck_core::config::elevenlabs::{
    self, ElevenLabsClient, VoiceConfig, ELEVENLABS_KEY_ID, MIN_UTTERANCE_MS, TTS_SAMPLE_RATE,
};

pub(super) struct AgentVoiceUi {
    /// A key is stored (the user may still have turned dictation off).
    has_key: bool,
    config: VoiceConfig,
    recording: Option<(Uuid, Recorder)>,
    transcribing: Option<Uuid>,
    speaker: Option<Speaker>,
    /// Repaints the button while recording (elapsed time, level).
    _meter: Option<Task<()>>,
}

impl AgentVoiceUi {
    pub(super) fn new() -> Self {
        Self {
            has_key: false,
            config: VoiceConfig::default(),
            recording: None,
            transcribing: None,
            speaker: None,
            _meter: None,
        }
    }
}

/// The key, read where it lives. `None`: no key stored.
fn stored_key() -> Result<String, String> {
    shelldeck_core::config::keychain::get_ai_api_key(ELEVENLABS_KEY_ID)
        .map_err(|error| error.to_string())?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| t!("agents.voice.no_key").to_string())
}

/// The reply to read aloud: the last agent message after the last user
/// message of the session.
pub(super) fn last_reply(messages: &[shelldeck_core::agent_session::AgentMessage]) -> Option<&str> {
    use shelldeck_core::agent_session::AgentMessageRole;
    messages
        .iter()
        .rev()
        .take_while(|message| message.role != AgentMessageRole::User)
        .find(|message| message.role == AgentMessageRole::Agent && !message.text.trim().is_empty())
        .map(|message| message.text.as_str())
}

impl AgentConsoleView {
    /// Workspace reports whether a key is stored and the `[voice]` choices.
    pub fn set_voice(&mut self, has_key: bool, config: VoiceConfig, cx: &mut Context<Self>) {
        self.voice.has_key = has_key;
        self.voice.config = config;
        if !has_key || !self.voice.config.read_replies {
            self.voice.speaker = None;
        }
        if !has_key || !self.voice.config.dictation {
            self.voice.recording = None;
            self.voice._meter = None;
        }
        cx.notify();
    }

    fn dictation_enabled(&self) -> bool {
        self.voice.has_key && self.voice.config.dictation
    }

    /// Microphone button: start, or stop and transcribe into the composer.
    fn toggle_dictation(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.voice.transcribing.is_some() {
            return;
        }
        if let Some((recording_for, _)) = &self.voice.recording {
            let recording_for = *recording_for;
            let (_, recorder) = self.voice.recording.take().expect("recording");
            self.voice._meter = None;
            self.transcribe(recording_for, recorder, cx);
            return;
        }
        // Speaking over the agent's voice would be transcribed too.
        self.voice.speaker = None;
        self.composer_ui.errors.remove(&session_id);
        match Recorder::start() {
            Ok(recorder) => {
                self.voice.recording = Some((session_id, recorder));
                self.voice._meter = Some(cx.spawn(async move |this, cx| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(120))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }));
            }
            Err(error) => {
                self.composer_ui.errors.insert(
                    session_id,
                    t!("agents.voice.mic_failed", error = error.to_string()).to_string(),
                );
            }
        }
        cx.notify();
    }

    fn transcribe(&mut self, session_id: Uuid, recorder: Recorder, cx: &mut Context<Self>) {
        self.voice.transcribing = Some(session_id);
        let config = self.voice.config.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let utterance = recorder.stop().map_err(|error| {
                        t!("agents.voice.mic_failed", error = error.to_string()).to_string()
                    })?;
                    if utterance.duration_ms < MIN_UTTERANCE_MS {
                        return Ok(None);
                    }
                    let key = stored_key()?;
                    let wav = elevenlabs::wav_from_pcm16(&utterance.samples, STT_SAMPLE_RATE);
                    ElevenLabsClient::new(&config.base_url, &key)
                        .and_then(|client| {
                            client.transcribe(wav, &config.stt_model, config.language.as_deref())
                        })
                        .map(Some)
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.voice.transcribing = None;
                match result {
                    Ok(Some(text)) if !text.is_empty() => {
                        view.append_to_prompt(session_id, &text, cx)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        view.composer_ui.errors.insert(session_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Dictated text goes after what is already typed; nothing is sent.
    fn append_to_prompt(&mut self, session_id: Uuid, text: &str, cx: &mut Context<Self>) {
        let Some(prompt) = self.session_ui.get(&session_id).map(|ui| ui.prompt.clone()) else {
            return;
        };
        prompt.update(cx, |state, cx| {
            let current = state.content().to_string();
            let joined = if current.trim().is_empty() {
                text.to_string()
            } else if current.ends_with(char::is_whitespace) {
                format!("{current}{text}")
            } else {
                format!("{current} {text}")
            };
            state.replace_content(joined, cx);
        });
    }

    /// A run finished cleanly: read its reply aloud when the user asked for it.
    pub(super) fn read_reply_aloud(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if !self.voice.has_key || !self.voice.config.read_replies {
            return;
        }
        let Some(text) = self
            .sessions
            .get(session_id)
            .and_then(|session| last_reply(&session.messages))
            .map(elevenlabs::spoken_text)
            .filter(|text| !text.is_empty())
        else {
            return;
        };
        let config = self.voice.config.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let key = stored_key()?;
                    let client = ElevenLabsClient::new(&config.base_url, &key)
                        .map_err(|error| error.to_string())?;
                    let voice = match config.voice_id.clone() {
                        Some(voice) => voice,
                        None => client
                            .voices()
                            .map_err(|error| error.to_string())?
                            .into_iter()
                            .next()
                            .map(|voice| voice.voice_id)
                            .ok_or_else(|| t!("agents.voice.no_voice").to_string())?,
                    };
                    let pcm = client
                        .speak(&text, &voice, &config.tts_model)
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(Speaker::play(pcm, TTS_SAMPLE_RATE))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    // Not while the user is dictating.
                    Ok(speaker) if view.voice.recording.is_none() => {
                        view.voice.speaker = Some(speaker)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        view.composer_ui.errors.insert(
                            session_id,
                            t!("agents.voice.read_failed", error = error).to_string(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Composer actions: the microphone (and, while a reply is read, a stop
    /// button). Nothing without a key or with both features off.
    pub(super) fn render_voice_actions(
        &self,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut actions = Vec::new();
        if self.voice.speaker.as_ref().is_some_and(Speaker::is_playing) {
            actions.push(
                Button::new("agent-voice-stop-reading", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Icon)
                    .h(px(30.0))
                    .icon(IconSource::from("volume-x"))
                    .tooltip(t!("agents.voice.stop_reading").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.voice.speaker = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        if !self.dictation_enabled() {
            return actions;
        }
        let recording = self
            .voice
            .recording
            .as_ref()
            .filter(|(id, _)| *id == session_id)
            .map(|(_, recorder)| (recorder.elapsed_secs(), recorder.level()));
        let transcribing = self.voice.transcribing == Some(session_id);
        let busy_elsewhere = (self.voice.recording.is_some() && recording.is_none())
            || (self.voice.transcribing.is_some() && !transcribing);
        let (label, icon, tooltip) = match (recording, transcribing) {
            (Some((secs, _)), _) => (
                format!("{}:{:02}", secs / 60, secs % 60),
                "square",
                t!("agents.voice.stop").to_string(),
            ),
            (None, true) => (
                t!("agents.voice.transcribing").to_string(),
                "mic",
                t!("agents.voice.transcribing").to_string(),
            ),
            (None, false) => (String::new(), "mic", t!("agents.voice.dictate").to_string()),
        };
        let mut button = Button::new("agent-voice-dictate", label)
            .variant(if recording.is_some() {
                ButtonVariant::Destructive
            } else {
                ButtonVariant::Ghost
            })
            .size(if recording.is_some() || transcribing {
                ButtonSize::Sm
            } else {
                ButtonSize::Icon
            })
            .h(px(30.0))
            .icon(IconSource::from(icon))
            .tooltip(tooltip)
            .disabled(transcribing || busy_elsewhere)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_dictation(session_id, cx)));
        if recording.is_some() {
            button = button.px(px(8.0));
        }
        let mut slot = div().flex().items_center().gap(px(4.0)).child(button);
        if let Some((_, level)) = recording {
            // The microphone hears: a small level bar beside the timer.
            slot = slot.child(
                div()
                    .w(px(28.0))
                    .h(px(4.0))
                    .rounded(px(2.0))
                    .bg(ShellDeckColors::border())
                    .child(
                        div()
                            .h_full()
                            .rounded(px(2.0))
                            .w(px((level.sqrt() * 28.0).clamp(2.0, 28.0)))
                            .bg(ShellDeckColors::success()),
                    ),
            );
        }
        actions.push(slot.into_any_element());
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shelldeck_core::agent_session::{AgentMessage, AgentMessageRole};
    use std::prelude::v1::test;

    fn message(role: AgentMessageRole, text: &str) -> AgentMessage {
        AgentMessage {
            id: Uuid::new_v4(),
            sequence: 0,
            turn_id: None,
            role,
            provider: None,
            text: text.into(),
            at_ms: 0,
        }
    }

    // SDTEST-3082: the reply read aloud is the latest agent message of the
    // last turn; an earlier turn's reply is never read for a turn that
    // produced none, and errors are not read.
    #[test]
    fn sdtest_3082_last_reply_belongs_to_the_last_turn() {
        use AgentMessageRole::*;
        let turn = vec![
            message(User, "first"),
            message(Agent, "old answer"),
            message(User, "second"),
            message(Agent, "working on it"),
            message(Agent, "done: tests pass"),
            message(Error, "warning"),
        ];
        assert_eq!(last_reply(&turn), Some("done: tests pass"));
        let silent = vec![
            message(User, "first"),
            message(Agent, "old"),
            message(User, "again"),
        ];
        assert_eq!(last_reply(&silent), None);
        assert_eq!(last_reply(&[message(Agent, "  ")]), None);
    }
}
