//! Text-to-speech backends: trait definition and implementations.
//!
//! The synthesis stage of the "read selection aloud" feature lives behind a
//! small [`TtsBackend`] trait (mirroring [`crate::transcription::TranscriptionBackend`])
//! so a local backend (piper/espeak) can be added later. v1 ships the Groq
//! backend only.

pub mod groq;

use async_trait::async_trait;

use crate::{TtsConfig, WhisrsError};

/// Trait for text-to-speech backends.
///
/// Each backend takes input text and returns synthesized speech as WAV bytes,
/// ready to be decoded and played by [`crate::audio::playback::play_wav`].
#[async_trait]
pub trait TtsBackend: Send + Sync {
    /// Synthesize `text` into speech, returning WAV-encoded audio bytes.
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, WhisrsError>;
}

/// Build the configured TTS backend.
///
/// `api_key` should already be resolved (TTS-specific key, falling back to the
/// Groq key / `WHISRS_GROQ_API_KEY`). Returns an error when no key is available.
/// v1 always builds the Groq backend.
pub fn create_backend(
    config: &TtsConfig,
    api_key: Option<String>,
) -> Result<Box<dyn TtsBackend>, WhisrsError> {
    let api_key = api_key.filter(|k| !k.is_empty()).ok_or_else(|| {
        WhisrsError::Config(
            "TTS is enabled but no API key is configured.\n\
             Add an api_key to [tts], or configure [groq] api_key / set WHISRS_GROQ_API_KEY."
                .to_string(),
        )
    })?;

    Ok(Box::new(groq::GroqTts::new(
        api_key,
        config.model.clone(),
        config.voice.clone(),
        config.response_format.clone(),
    )))
}
