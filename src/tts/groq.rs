//! Groq text-to-speech backend.
//!
//! Sends text to Groq's `/openai/v1/audio/speech` endpoint and returns the
//! synthesized speech as WAV-encoded audio bytes.

use async_trait::async_trait;
use serde::Serialize;
use tracing::debug;

use crate::WhisrsError;

use super::TtsBackend;

/// Groq API endpoint for text-to-speech.
const GROQ_SPEECH_URL: &str = "https://api.groq.com/openai/v1/audio/speech";

/// Groq text-to-speech backend.
pub struct GroqTts {
    client: reqwest::Client,
    api_key: String,
    model: String,
    voice: String,
    response_format: String,
}

impl GroqTts {
    /// Create a new Groq TTS backend.
    pub fn new(api_key: String, model: String, voice: String, response_format: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            voice,
            response_format,
        }
    }

    /// Build the JSON request body for the given input text.
    ///
    /// Exposed for unit testing the wire format without hitting the network.
    fn request_body<'a>(&'a self, text: &'a str) -> SpeechRequest<'a> {
        SpeechRequest {
            model: &self.model,
            voice: &self.voice,
            input: text,
            response_format: &self.response_format,
        }
    }
}

/// Request body for Groq's text-to-speech endpoint.
#[derive(Debug, Serialize)]
struct SpeechRequest<'a> {
    model: &'a str,
    voice: &'a str,
    input: &'a str,
    response_format: &'a str,
}

#[async_trait]
impl TtsBackend for GroqTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, WhisrsError> {
        if text.trim().is_empty() {
            return Err(WhisrsError::Transcription(
                "cannot synthesize empty text".to_string(),
            ));
        }

        debug!(
            "sending {} chars to Groq TTS (model={}, voice={}, format={})",
            text.len(),
            self.model,
            self.voice,
            self.response_format
        );

        let response = self
            .client
            .post(GROQ_SPEECH_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&self.request_body(text))
            .send()
            .await
            .map_err(|e| WhisrsError::Transcription(format!("Groq TTS request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(WhisrsError::Transcription(format!(
                "Groq TTS error ({}): {}",
                status.as_u16(),
                body
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| WhisrsError::Transcription(format!("Groq TTS read body failed: {e}")))?;

        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_serializes_expected_shape() {
        let backend = GroqTts::new(
            "test-key".to_string(),
            "canopylabs/orpheus-v1-english".to_string(),
            "autumn".to_string(),
            "wav".to_string(),
        );
        let json = serde_json::to_value(backend.request_body("hello world")).unwrap();
        assert_eq!(json["model"], "canopylabs/orpheus-v1-english");
        assert_eq!(json["voice"], "autumn");
        assert_eq!(json["input"], "hello world");
        assert_eq!(json["response_format"], "wav");
    }

    #[tokio::test]
    async fn synthesize_rejects_empty_text() {
        let backend = GroqTts::new(
            "test-key".to_string(),
            "canopylabs/orpheus-v1-english".to_string(),
            "autumn".to_string(),
            "wav".to_string(),
        );
        let err = backend.synthesize("   ").await.unwrap_err();
        assert!(err.to_string().contains("empty text"));
    }
}
