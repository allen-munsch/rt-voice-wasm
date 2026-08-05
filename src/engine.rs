//! Swappable STT engine trait.
//!
//! Implementors: Whisper (whisper.cpp), Moonshine (ONNX), etc.

/// A speech-to-text engine that transcribes 16kHz mono i16 PCM to text.
pub trait SttEngine: Send + Sync {
    /// Transcribe audio samples, returning one or more text segments.
    fn transcribe(&self, samples: &[i16]) -> Result<Vec<String>, String>;
}
