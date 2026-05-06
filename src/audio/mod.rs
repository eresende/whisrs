//! Audio capture and recovery.

pub mod capture;
pub mod feedback;
pub mod recovery;

/// A chunk of 16-bit PCM audio samples.
pub type AudioChunk = Vec<i16>;
