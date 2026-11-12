//! ElevenLabs client for speaking to agents with the user's own API key.
//!
//! Speech-to-text (`POST /v1/speech-to-text`, Scribe) turns a recorded
//! utterance into composer text; text-to-speech
//! (`POST /v1/text-to-speech/{voice}/stream`) reads agent replies aloud as raw
//! 16-bit PCM, so the desktop needs no audio decoder. The key travels only in
//! the `xi-api-key` header to the configured ElevenLabs host and is stored in
//! the OS keychain (`keychain::store_ai_api_key(ELEVENLABS_KEY_ID, …)`), never
//! in the configuration file or logs.

use crate::error::{Result, ShellDeckError};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;

/// Keychain slot of the user's key (`keychain::*_ai_api_key`).
pub const ELEVENLABS_KEY_ID: &str = "elevenlabs";
/// Public API host. Regional hosts can be configured instead.
pub const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
/// Speech-to-text model used unless the configuration names another one.
pub const DEFAULT_STT_MODEL: &str = "scribe_v2";
/// Text-to-speech model used unless the configuration names another one: the
/// low-latency Flash model (the same default as Monique's voice calls).
pub const DEFAULT_TTS_MODEL: &str = "eleven_flash_v2_5";
/// Sample rate of synthesized speech. 24 kHz PCM is available on every paid
/// tier (44.1 kHz needs Pro).
pub const TTS_SAMPLE_RATE: u32 = 24_000;
/// Longest text sent for one spoken reply. Longer replies are cut at a
/// sentence boundary before this many characters.
pub const MAX_TTS_CHARS: usize = 2_500;
/// Shortest utterance worth sending (ElevenLabs rejects under 100 ms).
pub const MIN_UTTERANCE_MS: u32 = 300;

/// `[voice]` settings: what the user chose, without the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Microphone button in the agent composer.
    pub dictation: bool,
    /// Read agent replies aloud when a run finishes.
    pub read_replies: bool,
    /// ElevenLabs voice for replies (`None`: the first voice of the account).
    pub voice_id: Option<String>,
    /// Display name of `voice_id`, for Settings without a network call.
    pub voice_name: Option<String>,
    pub tts_model: String,
    pub stt_model: String,
    /// ISO language hint for transcription (`None`: detected).
    pub language: Option<String>,
    pub base_url: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            dictation: true,
            read_replies: false,
            voice_id: None,
            voice_name: None,
            tts_model: DEFAULT_TTS_MODEL.to_string(),
            stt_model: DEFAULT_STT_MODEL.to_string(),
            language: None,
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }
}

/// The account behind a key, shown in Settings once the key is checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ElevenLabsAccount {
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub character_count: u64,
    #[serde(default)]
    pub character_limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ElevenLabsVoice {
    pub voice_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ElevenLabsModel {
    pub model_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub can_do_text_to_speech: bool,
}

pub struct ElevenLabsClient {
    base: String,
    key: String,
    http: reqwest::blocking::Client,
}

impl ElevenLabsClient {
    pub fn new(base_url: &str, key: &str) -> Result<Self> {
        let base = base_url.trim().trim_end_matches('/').to_string();
        let parsed = reqwest::Url::parse(&base)
            .map_err(|_| ShellDeckError::Config("ElevenLabs address is invalid".into()))?;
        let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"));
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
            return Err(ShellDeckError::Config(
                "ElevenLabs address must use https".into(),
            ));
        }
        let key = key.trim().to_string();
        if key.is_empty() || key.len() > 256 || key.chars().any(char::is_whitespace) {
            return Err(ShellDeckError::Config("ElevenLabs key is invalid".into()));
        }
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(6))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| ShellDeckError::Config(error.to_string()))?;
        Ok(Self { base, key, http })
    }

    fn get(&self, path: &str) -> Result<reqwest::blocking::Response> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("xi-api-key", &self.key)
            .send()
            .map_err(network)?;
        checked(response)
    }

    /// Check the key and read the subscription it belongs to.
    pub fn account(&self) -> Result<ElevenLabsAccount> {
        #[derive(Deserialize)]
        struct User {
            #[serde(default)]
            subscription: ElevenLabsAccount,
        }
        let user: User = self.get("/v1/user")?.json().map_err(decode)?;
        Ok(user.subscription)
    }

    /// The account's voices (premade, cloned, library), in API order.
    pub fn voices(&self) -> Result<Vec<ElevenLabsVoice>> {
        #[derive(Deserialize)]
        struct Voices {
            #[serde(default)]
            voices: Vec<ElevenLabsVoice>,
        }
        let list: Voices = self.get("/v1/voices")?.json().map_err(decode)?;
        Ok(list.voices)
    }

    /// Models that can speak.
    pub fn tts_models(&self) -> Result<Vec<ElevenLabsModel>> {
        let models: Vec<ElevenLabsModel> = self.get("/v1/models")?.json().map_err(decode)?;
        Ok(models
            .into_iter()
            .filter(|model| model.can_do_text_to_speech)
            .collect())
    }

    /// Transcribe one recorded utterance (`wav`: a complete WAV file).
    pub fn transcribe(&self, wav: Vec<u8>, model: &str, language: Option<&str>) -> Result<String> {
        #[derive(Deserialize)]
        struct Transcript {
            #[serde(default)]
            text: String,
        }
        let file = reqwest::blocking::multipart::Part::bytes(wav)
            .file_name("utterance.wav")
            .mime_str("audio/wav")
            .map_err(|error| ShellDeckError::Config(error.to_string()))?;
        let mut form = reqwest::blocking::multipart::Form::new()
            .text("model_id", model.to_string())
            .text("tag_audio_events", "false")
            .part("file", file);
        if let Some(language) = language.filter(|code| !code.trim().is_empty()) {
            form = form.text("language_code", language.trim().to_string());
        }
        let response = self
            .http
            .post(format!("{}/v1/speech-to-text", self.base))
            .header("xi-api-key", &self.key)
            .multipart(form)
            .send()
            .map_err(network)?;
        let transcript: Transcript = checked(response)?.json().map_err(decode)?;
        Ok(transcript.text.trim().to_string())
    }

    /// Start speaking `text`: a reader of mono 16-bit little-endian PCM at
    /// [`TTS_SAMPLE_RATE`], streamed as ElevenLabs produces it.
    pub fn speak(&self, text: &str, voice_id: &str, model: &str) -> Result<impl Read + Send> {
        let voice = voice_id.trim();
        if voice.is_empty() || !voice.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ShellDeckError::Config("ElevenLabs voice is invalid".into()));
        }
        let response = self
            .http
            .post(format!(
                "{}/v1/text-to-speech/{voice}/stream?output_format=pcm_{TTS_SAMPLE_RATE}",
                self.base
            ))
            .header("xi-api-key", &self.key)
            .json(&serde_json::json!({
                "text": spoken_text(text),
                "model_id": model,
            }))
            .send()
            .map_err(network)?;
        checked(response)
    }
}

fn network(error: reqwest::Error) -> ShellDeckError {
    // Never include the request (and its headers) in the message.
    ShellDeckError::Config(format!("ElevenLabs unreachable: {}", error.without_url()))
}

fn decode(error: reqwest::Error) -> ShellDeckError {
    ShellDeckError::Config(format!(
        "ElevenLabs answer not understood: {}",
        error.without_url()
    ))
}

/// Turn an error status into a message that says what to do.
fn checked(response: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let detail = response
        .text()
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|value| {
            value
                .pointer("/detail/message")
                .or_else(|| value.pointer("/detail"))
                .and_then(|detail| detail.as_str().map(str::to_string))
        })
        .map(|detail| detail.chars().take(200).collect::<String>())
        .unwrap_or_default();
    let reason = match status.as_u16() {
        401 => "key refused (check or replace it in Settings)".to_string(),
        402 | 429 => "quota or rate limit reached on this ElevenLabs account".to_string(),
        code => format!("HTTP {code}"),
    };
    Err(ShellDeckError::Config(if detail.is_empty() {
        format!("ElevenLabs: {reason}")
    } else {
        format!("ElevenLabs: {reason} — {detail}")
    }))
}

/// What is read aloud from an agent reply: Markdown markers, code blocks and
/// links reduced to their words, cut at a sentence boundary under
/// [`MAX_TTS_CHARS`].
pub fn spoken_text(reply: &str) -> String {
    let mut out = String::with_capacity(reply.len().min(MAX_TTS_CHARS + 16));
    let mut in_code = false;
    for line in reply.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || trimmed.is_empty() {
            continue;
        }
        let line = trimmed.trim_start_matches(['#', '>', '-', '*']).trim();
        let mut cleaned = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' => {}
                // [label](url) → label
                ']' if chars.peek() == Some(&'(') => {
                    for next in chars.by_ref() {
                        if next == ')' {
                            break;
                        }
                    }
                }
                '[' => {}
                _ => cleaned.push(c),
            }
        }
        if !out.is_empty() {
            out.push(if out.ends_with(['.', '!', '?', ':']) {
                ' '
            } else {
                '.'
            });
            if !out.ends_with(' ') {
                out.push(' ');
            }
        }
        out.push_str(cleaned.trim());
    }
    if out.chars().count() <= MAX_TTS_CHARS {
        return out;
    }
    let cut: String = out.chars().take(MAX_TTS_CHARS).collect();
    match cut.rfind(['.', '!', '?']) {
        Some(end) if end > MAX_TTS_CHARS / 3 => cut[..=end].to_string(),
        _ => cut,
    }
}

/// A WAV file of mono 16-bit PCM, as sent for transcription.
pub fn wav_from_pcm16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

    /// One-request-per-connection HTTP mock: (request line, xi-api-key, body).
    fn mock(answer: impl Fn(&str) -> (u16, Vec<u8>) + Send + 'static) -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen: Seen = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let (mut key, mut length) = (String::new(), 0usize);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (name, value) = header.split_once(':').unwrap();
                    match name.to_ascii_lowercase().as_str() {
                        "xi-api-key" => key = value.trim().to_string(),
                        "content-length" => length = value.trim().parse().unwrap(),
                        _ => {}
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let target = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let (status, payload) = answer(&target);
                log.lock().unwrap().push((target, key, body));
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                )
                .unwrap();
                stream.write_all(&payload).unwrap();
            }
        });
        (format!("http://{address}"), seen)
    }

    // SDTEST-3079: the ElevenLabs client sends the key only as `xi-api-key`,
    // reads the account, voices and speaking models, sends a WAV for
    // transcription with the model and language, streams raw PCM back for a
    // reply, and turns refusals into a message without the key.
    #[test]
    fn sdtest_3079_elevenlabs_client_contract() {
        let (base, seen) = mock(|target| {
            match target {
            "/v1/user" => (200, br#"{"subscription":{"tier":"creator","character_count":12,"character_limit":100000}}"#.to_vec()),
            "/v1/voices" => (200, br#"{"voices":[{"voice_id":"abc123","name":"Rachel","category":"premade"}]}"#.to_vec()),
            "/v1/models" => (200, br#"[{"model_id":"eleven_multilingual_v2","name":"Multilingual v2","can_do_text_to_speech":true},{"model_id":"scribe_v2","can_do_text_to_speech":false}]"#.to_vec()),
            "/v1/speech-to-text" => (200, br#"{"text":"  ouvre le fichier main.rs  ","language_code":"fra"}"#.to_vec()),
            t if t.starts_with("/v1/text-to-speech/abc123/stream?output_format=pcm_24000") => (200, vec![1, 0, 2, 0, 3, 0]),
            _ => (401, br#"{"detail":{"status":"invalid_api_key","message":"Invalid API key"}}"#.to_vec()),
        }
        });
        let client = ElevenLabsClient::new(&base, "sk_test_123").unwrap();
        let account = client.account().unwrap();
        assert_eq!(account.tier, "creator");
        assert_eq!(account.character_limit, 100_000);
        assert_eq!(client.voices().unwrap()[0].name, "Rachel");
        let models = client.tts_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "eleven_multilingual_v2");

        let wav = wav_from_pcm16(&[0, 100, -100], 16_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(
            client.transcribe(wav, "scribe_v2", Some("fr")).unwrap(),
            "ouvre le fichier main.rs"
        );
        let mut pcm = Vec::new();
        client
            .speak("**Bonjour** !", "abc123", "eleven_multilingual_v2")
            .unwrap()
            .read_to_end(&mut pcm)
            .unwrap();
        assert_eq!(pcm, vec![1, 0, 2, 0, 3, 0]);
        assert!(
            client.speak("x", "../etc", "m").is_err(),
            "voice id is a path segment"
        );

        {
            let seen = seen.lock().unwrap();
            assert!(seen.iter().all(|(_, key, _)| key == "sk_test_123"));
            let stt = seen
                .iter()
                .find(|(t, _, _)| t == "/v1/speech-to-text")
                .unwrap();
            let body = String::from_utf8_lossy(&stt.2);
            assert!(body.contains("name=\"model_id\"\r\n\r\nscribe_v2"));
            assert!(body.contains("name=\"language_code\"\r\n\r\nfr"));
            assert!(body.contains("filename=\"utterance.wav\""));
            let tts = seen
                .iter()
                .find(|(t, _, _)| t.contains("text-to-speech"))
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&tts.2).unwrap();
            assert_eq!(json["text"], "Bonjour !");
            assert_eq!(json["model_id"], "eleven_multilingual_v2");
        }

        let refused = ElevenLabsClient::new(&base, "sk_test_123")
            .unwrap()
            .get("/v1/other")
            .err()
            .unwrap()
            .to_string();
        assert!(refused.contains("key refused") && refused.contains("Invalid API key"));
        assert!(!refused.contains("sk_test_123"));

        assert!(ElevenLabsClient::new("http://api.example.test", "k").is_err());
        assert!(ElevenLabsClient::new(DEFAULT_BASE_URL, "  ").is_err());
        assert!(ElevenLabsClient::new(DEFAULT_BASE_URL, "a b").is_err());
    }

    // SDTEST-3080: what is read aloud from a reply: no code blocks, no
    // Markdown markers, link labels without URLs, lines joined as
    // sentences, long replies cut at a sentence end.
    #[test]
    fn sdtest_3080_spoken_text_of_a_reply() {
        let reply = "## Résumé\n\nJ'ai corrigé **deux** bugs dans `main.rs`\n\n```rust\nfn main() {}\n```\n- Voir [la PR](https://github.com/x/y/pull/1)\n> fini !";
        assert_eq!(
            spoken_text(reply),
            "Résumé. J'ai corrigé deux bugs dans main.rs. Voir la PR. fini !"
        );
        let long = "Une phrase assez longue pour remplir. ".repeat(200);
        let spoken = spoken_text(&long);
        assert!(spoken.chars().count() <= MAX_TTS_CHARS);
        assert!(spoken.ends_with('.'));
    }
}
