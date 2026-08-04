#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TimingBreakdown {
    pub encoder_ms: f64,
    pub decoder_ms: f64,
    pub total_ms: f64,
}

pub struct WhisperContext;

impl WhisperContext {
    pub fn init_from_file(_path: &str) -> Result<Self, String> {
        Err("whisper not available on wasm32 — use native host target".into())
    }

    pub fn transcribe(&self, _samples: &[i16]) -> Result<(Vec<Segment>, TimingBreakdown), String> {
        Err("whisper not available on wasm32".into())
    }
}
