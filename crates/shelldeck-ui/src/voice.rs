//! Microphone capture and speech playback for talking to agents.
//!
//! Neither a `cpal` input stream nor a `rodio` output stream is `Send`, so each
//! lives on its own thread and is driven through channels:
//!
//! - [`Recorder`]: push-to-talk. `start` opens the default input device;
//!   `stop` returns the utterance as mono 16 kHz PCM (what ElevenLabs Scribe
//!   is sent, in a WAV); `level` gives a peak meter for the button.
//! - [`Speaker`]: plays mono 16-bit PCM as it streams in (ElevenLabs
//!   `pcm_24000`); dropping or `stop` silences it at once.
//!
//! Nothing here touches the network or the key.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// Rate of what is sent for transcription.
pub const STT_SAMPLE_RATE: u32 = 16_000;
/// Longest utterance kept (ElevenLabs bills by duration; push-to-talk turns
/// are short). Later audio is dropped.
pub const MAX_UTTERANCE_SECS: u32 = 120;

/// Why audio could not start, in words for a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioError(pub String);

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A finished utterance.
pub struct Utterance {
    /// Mono 16-bit PCM at [`STT_SAMPLE_RATE`].
    pub samples: Vec<i16>,
    pub duration_ms: u32,
}

/// One push-to-talk recording in progress.
pub struct Recorder {
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<Result<Utterance, AudioError>>>,
    peak: Arc<AtomicU32>,
    started: Instant,
}

impl Recorder {
    /// Open the default microphone and start recording. Returns once the
    /// device is open (or failed to open).
    pub fn start() -> Result<Self, AudioError> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let peak = Arc::new(AtomicU32::new(0));
        let meter = peak.clone();
        let thread = std::thread::Builder::new()
            .name("voice-recorder".into())
            .spawn(move || record(stop_rx, ready_tx, meter))
            .map_err(|error| AudioError(error.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop: stop_tx,
                thread: Some(thread),
                peak,
                started: Instant::now(),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(AudioError("microphone thread ended".into())),
        }
    }

    /// Recent input peak, 0.0–1.0, for a level meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Stop and return what was said.
    pub fn stop(mut self) -> Result<Utterance, AudioError> {
        let _ = self.stop.send(());
        self.thread
            .take()
            .expect("recorder thread")
            .join()
            .map_err(|_| AudioError("microphone thread panicked".into()))?
    }
}

impl Drop for Recorder {
    /// Dropped without `stop` (cancel): close the device, discard the audio.
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn record(
    stop: mpsc::Receiver<()>,
    ready: mpsc::Sender<Result<(), AudioError>>,
    peak: Arc<AtomicU32>,
) -> Result<Utterance, AudioError> {
    let fail = |ready: &mpsc::Sender<Result<(), AudioError>>, message: String| {
        let error = AudioError(message);
        let _ = ready.send(Err(error.clone()));
        Err(error)
    };
    let host = cpal::default_host();
    let Some(device) = host.default_input_device() else {
        return fail(&ready, "no microphone found".into());
    };
    let config = match device.default_input_config() {
        Ok(config) => config,
        Err(error) => return fail(&ready, format!("microphone unavailable: {error}")),
    };
    let rate = config.sample_rate().0;
    let channels = config.channels().max(1) as usize;
    let cap = (rate as usize) * channels * MAX_UTTERANCE_SECS as usize;
    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 4)));
    let sink = buffer.clone();
    let push = move |data: &mut dyn Iterator<Item = f32>| {
        let mut buffer = sink.lock().unwrap();
        let mut loudest = 0.0f32;
        for sample in data {
            loudest = loudest.max(sample.abs());
            if buffer.len() < cap {
                buffer.push(sample);
            }
        }
        // A decaying meter: the button shows that the microphone hears.
        let previous = f32::from_bits(peak.load(Ordering::Relaxed));
        peak.store(
            (previous * 0.8).max(loudest).min(1.0).to_bits(),
            Ordering::Relaxed,
        );
    };
    let on_error = |error| tracing::warn!("microphone stream error: {error}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _: &_| push(&mut data.iter().copied()),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data: &[i16], _: &_| push(&mut data.iter().map(|s| *s as f32 / i16::MAX as f32)),
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config.into(),
            move |data: &[u16], _: &_| {
                push(&mut data.iter().map(|s| (*s as f32 - 32_768.0) / 32_768.0))
            },
            on_error,
            None,
        ),
        other => return fail(&ready, format!("unsupported microphone format {other:?}")),
    };
    let stream = match stream {
        Ok(stream) => stream,
        Err(error) => return fail(&ready, format!("microphone unavailable: {error}")),
    };
    if let Err(error) = stream.play() {
        return fail(&ready, format!("microphone unavailable: {error}"));
    }
    let _ = ready.send(Ok(()));
    let _ = stop.recv();
    drop(stream);
    let raw = std::mem::take(&mut *buffer.lock().unwrap());
    let mono = downmix(&raw, channels);
    let samples = resample_to_i16(&mono, rate, STT_SAMPLE_RATE);
    let duration_ms = (samples.len() as u64 * 1000 / STT_SAMPLE_RATE as u64) as u32;
    Ok(Utterance {
        samples,
        duration_ms,
    })
}

/// Average interleaved channels into one.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Linear resampling to `to` Hz, as 16-bit samples. Speech for transcription
/// needs no better filter.
pub fn resample_to_i16(mono: &[f32], from: u32, to: u32) -> Vec<i16> {
    if mono.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    let ratio = from as f64 / to as f64;
    let len = ((mono.len() as f64) / ratio).floor() as usize;
    (0..len)
        .map(|index| {
            let position = index as f64 * ratio;
            let base = position.floor() as usize;
            let next = (base + 1).min(mono.len() - 1);
            let fraction = (position - base as f64) as f32;
            let sample = mono[base] * (1.0 - fraction) + mono[next] * fraction;
            (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
        })
        .collect()
}

/// A reply being read aloud. Dropping it stops the voice.
pub struct Speaker {
    stopped: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl Speaker {
    /// Play mono 16-bit little-endian PCM at `sample_rate` from `pcm`, as it
    /// arrives. Returns at once; errors are logged (the reply is still on
    /// screen).
    pub fn play(pcm: impl Read + Send + 'static, sample_rate: u32) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let (stop_flag, done_flag) = (stopped.clone(), finished.clone());
        let spawned = std::thread::Builder::new()
            .name("voice-speaker".into())
            .spawn(move || {
                if let Err(error) = speak(pcm, sample_rate, &stop_flag) {
                    tracing::warn!("reading the reply aloud failed: {error}");
                }
                done_flag.store(true, Ordering::Relaxed);
            });
        if spawned.is_err() {
            finished.store(true, Ordering::Relaxed);
        }
        Self { stopped, finished }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Still speaking (or about to).
    pub fn is_playing(&self) -> bool {
        !self.finished.load(Ordering::Relaxed) && !self.stopped.load(Ordering::Relaxed)
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn speak(mut pcm: impl Read, sample_rate: u32, stopped: &AtomicBool) -> Result<(), AudioError> {
    let (_stream, handle) = rodio::OutputStream::try_default()
        .map_err(|error| AudioError(format!("no audio output: {error}")))?;
    let sink = rodio::Sink::try_new(&handle).map_err(|error| AudioError(error.to_string()))?;
    // ~100 ms per chunk keeps latency low without starving the sink.
    let mut chunk = vec![0u8; (sample_rate as usize / 10) * 2];
    let mut carry: Option<u8> = None;
    loop {
        if stopped.load(Ordering::Relaxed) {
            sink.stop();
            return Ok(());
        }
        let read = pcm
            .read(&mut chunk)
            .map_err(|error| AudioError(error.to_string()))?;
        if read == 0 {
            break;
        }
        let (samples, rest) = pcm16_le(carry.take(), &chunk[..read]);
        carry = rest;
        if !samples.is_empty() {
            sink.append(rodio::buffer::SamplesBuffer::new(1, sample_rate, samples));
        }
    }
    while !sink.empty() {
        if stopped.load(Ordering::Relaxed) {
            sink.stop();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

/// Decode little-endian 16-bit samples from a byte chunk, carrying an odd
/// trailing byte over to the next chunk (network reads split anywhere).
pub fn pcm16_le(carry: Option<u8>, bytes: &[u8]) -> (Vec<i16>, Option<u8>) {
    let mut joined;
    let bytes = match carry {
        Some(first) => {
            joined = Vec::with_capacity(bytes.len() + 1);
            joined.push(first);
            joined.extend_from_slice(bytes);
            &joined[..]
        }
        None => bytes,
    };
    let samples = bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let rest = (bytes.len() % 2 == 1).then(|| bytes[bytes.len() - 1]);
    (samples, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-3081: audio helpers. Stereo frames are averaged, 48 kHz becomes
    // 16 kHz with a third of the samples and the same shape, full scale maps
    // to i16 bounds, and PCM split at an odd byte is reassembled.
    #[test]
    fn sdtest_3081_audio_conversions() {
        assert_eq!(downmix(&[1.0, 0.0, -1.0, -1.0], 2), vec![0.5, -1.0]);
        assert_eq!(downmix(&[0.25, 0.5], 1), vec![0.25, 0.5]);

        let ramp: Vec<f32> = (0..48).map(|i| i as f32 / 48.0).collect();
        let resampled = resample_to_i16(&ramp, 48_000, 16_000);
        assert_eq!(resampled.len(), 16);
        assert!(resampled.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            resample_to_i16(&[1.5, -1.5], 16_000, 16_000),
            vec![i16::MAX, -i16::MAX]
        );
        assert!(resample_to_i16(&[], 48_000, 16_000).is_empty());

        let bytes = [0x01, 0x00, 0xff, 0x7f, 0x00];
        let (first, carry) = pcm16_le(None, &bytes);
        assert_eq!(first, vec![1, i16::MAX]);
        assert_eq!(carry, Some(0x00));
        let (second, carry) = pcm16_le(carry, &[0x80]);
        assert_eq!(second, vec![i16::MIN]);
        assert_eq!(carry, None);
    }
}
