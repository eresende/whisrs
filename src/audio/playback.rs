//! Interruptible WAV playback on the default audio output device.
//!
//! Used by the read-selection-aloud feature to play TTS audio of arbitrary
//! sample rate / channel count. Unlike [`crate::audio::feedback`] (mono /
//! 44.1 kHz / 2-second timeout), this plays the clip to completion and can be
//! stopped early via a shared [`AtomicBool`].

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, StreamConfig};
use tracing::{debug, warn};

use crate::WhisrsError;

/// Decoded PCM audio: interleaved f32 samples plus the stream format.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedWav {
    /// Interleaved samples in `[-1.0, 1.0]`, channel-major per frame.
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
}

impl DecodedWav {
    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
}

/// Decode WAV bytes into interleaved f32 samples plus format metadata.
///
/// Handles 16-bit integer and 32-bit float WAV files (the formats Groq's TTS
/// endpoint returns); other integer bit depths (8/24/32) are also supported by
/// scaling to f32. This is a pure function with no audio device access so it
/// can be unit-tested.
pub fn decode_wav(wav_bytes: &[u8]) -> Result<DecodedWav, WhisrsError> {
    let reader = hound::WavReader::new(Cursor::new(wav_bytes))
        .map_err(|e| WhisrsError::Audio(format!("failed to read WAV header: {e}")))?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| WhisrsError::Audio(format!("failed to decode float WAV samples: {e}")))?,
        hound::SampleFormat::Int => {
            // Normalize integer samples by the full-scale value for the bit depth.
            let max_amp = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max_amp))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    WhisrsError::Audio(format!("failed to decode integer WAV samples: {e}"))
                })?
        }
    };

    if spec.channels == 0 {
        return Err(WhisrsError::Audio("WAV reports zero channels".to_string()));
    }

    Ok(DecodedWav {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    })
}

/// Play WAV-encoded audio on the default output device, blocking until the clip
/// finishes or `stop` is set to `true`.
///
/// Builds a cpal output stream matching the WAV's sample rate and channel count.
/// The stream callback advances through the decoded samples; when they are
/// exhausted (or `stop` is set) it emits silence and signals completion. This
/// function is intended to be run on a blocking task (`spawn_blocking`).
pub fn play_wav(wav_bytes: &[u8], stop: Arc<AtomicBool>) -> Result<(), WhisrsError> {
    let decoded = decode_wav(wav_bytes)?;
    play_decoded(decoded, stop)
}

/// Play already-decoded PCM on the default output device. See [`play_wav`].
fn play_decoded(decoded: DecodedWav, stop: Arc<AtomicBool>) -> Result<(), WhisrsError> {
    if decoded.samples.is_empty() {
        return Ok(());
    }

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| WhisrsError::Audio("no default audio output device".to_string()))?;

    let config = StreamConfig {
        channels: decoded.channels,
        sample_rate: SampleRate(decoded.sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let samples = decoded.samples;
    let samples_len = samples.len();
    let sample_idx = Arc::new(AtomicUsize::new(0));
    let sample_idx_cb = Arc::clone(&sample_idx);
    let done = Arc::new(AtomicBool::new(false));
    let done_cb = Arc::clone(&done);
    let stop_cb = Arc::clone(&stop);

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                if stop_cb.load(Ordering::Acquire) {
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                    done_cb.store(true, Ordering::Release);
                    return;
                }
                for sample in data.iter_mut() {
                    let idx = sample_idx_cb.fetch_add(1, Ordering::Relaxed);
                    if idx < samples_len {
                        *sample = samples[idx];
                    } else {
                        *sample = 0.0;
                        done_cb.store(true, Ordering::Release);
                    }
                }
            },
            |err| {
                warn!("TTS playback stream error: {err}");
            },
            None,
        )
        .map_err(|e| WhisrsError::Audio(format!("failed to build output stream: {e}")))?;

    stream
        .play()
        .map_err(|e| WhisrsError::Audio(format!("failed to start playback: {e}")))?;

    // Compute a generous upper bound on playback duration so a stuck stream
    // can't block forever.
    let frames = (samples_len / decoded.channels.max(1) as usize) as f64;
    let clip_secs = frames / decoded.sample_rate.max(1) as f64;
    let timeout = Duration::from_secs_f64(clip_secs + 2.0);
    let start = Instant::now();

    while !done.load(Ordering::Acquire) {
        if stop.load(Ordering::Acquire) {
            debug!("TTS playback interrupted");
            break;
        }
        if start.elapsed() > timeout {
            debug!("TTS playback timed out after {:.1}s", timeout.as_secs_f64());
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // Let the final buffer drain.
    std::thread::sleep(Duration::from_millis(50));
    drop(stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a short stereo i16 tone WAV in memory for decode round-trip tests.
    fn make_i16_wav(sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buf = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut buf, spec).unwrap();
            for i in 0..frames {
                let v = ((i as f32 * 0.01).sin() * i16::MAX as f32) as i16;
                for _ in 0..channels {
                    writer.write_sample(v).unwrap();
                }
            }
            writer.finalize().unwrap();
        }
        buf.into_inner()
    }

    fn make_f32_wav(sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut buf = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut buf, spec).unwrap();
            for i in 0..frames {
                let v = (i as f32 * 0.01).sin() * 0.5;
                for _ in 0..channels {
                    writer.write_sample(v).unwrap();
                }
            }
            writer.finalize().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn decode_i16_stereo_wav() {
        let wav = make_i16_wav(24_000, 2, 100);
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.sample_rate, 24_000);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples.len(), 200);
        assert_eq!(decoded.frames(), 100);
        // Normalized into [-1.0, 1.0].
        assert!(decoded.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn decode_i16_mono_wav() {
        let wav = make_i16_wav(16_000, 1, 50);
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.sample_rate, 16_000);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples.len(), 50);
        assert_eq!(decoded.frames(), 50);
    }

    #[test]
    fn decode_f32_wav() {
        let wav = make_f32_wav(44_100, 1, 64);
        let decoded = decode_wav(&wav).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples.len(), 64);
    }

    #[test]
    fn decode_rejects_garbage() {
        let err = decode_wav(b"not a wav file at all").unwrap_err();
        assert!(err.to_string().contains("WAV header"));
    }
}
