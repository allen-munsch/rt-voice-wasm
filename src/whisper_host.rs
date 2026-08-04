use std::ffi::{CStr, CString};
use std::time::Instant;

include!(concat!(env!("OUT_DIR"), "/whisper_bindings.rs"));

#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
}

pub struct WhisperContext {
    inner: *mut whisper_context,
}

#[derive(Debug, Clone, Default)]
pub struct TimingBreakdown {
    pub encoder_ms: f64,
    pub decoder_ms: f64,
    pub total_ms: f64,
}

unsafe impl Send for WhisperContext {}
unsafe impl Sync for WhisperContext {}

impl WhisperContext {
    pub fn init_from_file(path: &str) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|e| format!("invalid path: {e}"))?;
        let params = unsafe { whisper_context_default_params() };
        let inner = unsafe { whisper_init_from_file_with_params(c_path.as_ptr(), params) };
        if inner.is_null() {
            return Err(format!("failed to load model from {path}"));
        }
        Ok(WhisperContext { inner })
    }

    pub fn transcribe(&self, samples: &[i16]) -> Result<(Vec<Segment>, TimingBreakdown), String> {
        let f32_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
        let n_samples = f32_samples.len() as i32;
        let audio_duration_s = n_samples as f64 / 16000.0;

        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(1);
        let mut params = unsafe {
            whisper_full_default_params(whisper_sampling_strategy_WHISPER_SAMPLING_GREEDY)
        };
        params.n_threads = n_threads.min(8);
        params.print_progress = false;
        params.print_realtime = false;
        params.print_timestamps = false;
        params.no_timestamps = true;
        params.single_segment = true;
        params.suppress_blank = true;
        params.suppress_nst = true;
        params.temperature = 0.0f32;
        params.language = b"en\0".as_ptr() as *const std::ffi::c_char;

        // Limit encoder to only the audio we actually have, not full 30s context
        let remaining_s = (audio_duration_s.ceil() as i32).max(1);
        params.duration_ms = (remaining_s * 1000).min(30_000);

        unsafe { whisper_reset_timings(self.inner) };
        let t0 = Instant::now();
        let ret = unsafe { whisper_full(self.inner, params, f32_samples.as_ptr(), n_samples) };
        let total_ms = t0.elapsed().as_secs_f64() * 1000.0;
        if ret != 0 {
            return Err(format!("whisper_full returned error code {ret}"));
        }

        let n_segments = unsafe { whisper_full_n_segments(self.inner) };
        let mut segments = Vec::with_capacity(n_segments as usize);
        for i in 0..n_segments {
            let text_ptr = unsafe { whisper_full_get_segment_text(self.inner, i) };
            let text = if text_ptr.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(text_ptr) }.to_string_lossy().into_owned()
            };
            let t0 = unsafe { whisper_full_get_segment_t0(self.inner, i) };
            let t1 = unsafe { whisper_full_get_segment_t1(self.inner, i) };
            segments.push(Segment {
                text,
                t0_ms: t0 * 10,
                t1_ms: t1 * 10,
            });
        }

        let timing = TimingBreakdown {
            total_ms,
            ..Default::default()
        };
        Ok((segments, timing))
    }
}

impl Drop for WhisperContext {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe { whisper_free(self.inner) };
        }
    }
}
