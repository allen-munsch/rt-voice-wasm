use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatencyStats {
    pub inference_ms: f64,
    pub audio_duration_s: f64,
    pub rtf: f64,
}

pub struct StreamingPipeline {
    #[allow(dead_code)]
    sample_rate: u32,
    window_samples: usize,
    #[allow(dead_code)]
    step_samples: usize,
    buffer: Vec<i16>,
    rms_threshold: f64,
    last_output: String,
    speed_factor: f64,
}

impl StreamingPipeline {
    pub fn new(sample_rate: u32) -> Self {
        Self::with_speed(sample_rate, 1.0)
    }

    pub fn with_speed(sample_rate: u32, speed_factor: f64) -> Self {
        Self::with_params(sample_rate, speed_factor, 3.0, 1.0)
    }

    pub fn with_params(sample_rate: u32, speed_factor: f64, window_secs: f64, step_secs: f64) -> Self {
        let window_samples = (sample_rate as f64 * window_secs) as usize;
        let step_samples = (sample_rate as f64 * step_secs) as usize;
        StreamingPipeline {
            sample_rate,
            window_samples,
            step_samples,
            buffer: Vec::new(),
            rms_threshold: 0.01,
            last_output: String::new(),
            speed_factor,
        }
    }

    pub fn push_frame(&mut self, samples: &[i16]) -> Option<Vec<i16>> {
        self.buffer.extend_from_slice(samples);
        if self.buffer.len() >= self.window_samples {
            let window: Vec<i16> = self.buffer[self.buffer.len() - self.window_samples..].to_vec();
            // Slide: discard oldest step_samples, keep the overlap
            // split_off(n) returns [n..len); we want [step_samples..len]
            self.buffer = self.buffer.split_off(self.step_samples as usize);
            let rms = self.compute_rms(&window);
            if rms < self.rms_threshold {
                return None;
            }
            Some(window)
        } else {
            None
        }
    }

    fn compute_rms(&self, samples: &[i16]) -> f64 {
        let sum_sq: f64 = samples.iter().map(|&s| s as f64 * s as f64).sum();
        (sum_sq / samples.len() as f64).sqrt() / 32768.0
    }

    pub fn merge_overlap(&mut self, new_text: &str) -> String {
        if new_text.is_empty() {
            return String::new();
        }
        if self.last_output.is_empty() {
            self.last_output = new_text.to_string();
            return new_text.to_string();
        }
        let words: Vec<&str> = new_text.split_whitespace().collect();
        let prev_words: Vec<&str> = self.last_output.split_whitespace().collect();
        let mut best_overlap = 0usize;
        for overlap_len in 1..=words.len().min(prev_words.len()).min(8) {
            let w_tail: Vec<&str> = words[..overlap_len].iter().map(|s| *s).collect();
            let p_tail: Vec<&str> = prev_words[prev_words.len() - overlap_len..].iter().map(|s| *s).collect();
            let matched = w_tail.iter().zip(&p_tail).all(|(a, b)| {
                a.eq_ignore_ascii_case(b)
            });
            if matched {
                best_overlap = overlap_len;
            }
        }
        let merged = if best_overlap > 0 {
            words[best_overlap..].join(" ")
        } else {
            new_text.to_string()
        };
        self.last_output = new_text.to_string();
        merged
    }

    pub fn last_output(&self) -> &str {
        &self.last_output
    }

    pub fn speed_factor(&self) -> f64 {
        self.speed_factor
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// Flush any remaining buffered audio as a partial window.
    /// Called when the transport disconnects to avoid dropping trailing speech.
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        if self.buffer.is_empty() {
            return None;
        }
        // Pad to full window with silence so whisper gets enough context
        let mut window = self.buffer.clone();
        if window.len() < self.window_samples {
            window.resize(self.window_samples, 0i16);
        }
        self.buffer.clear();
        let rms = self.compute_rms(&window[..self.window_samples]);
        if rms < self.rms_threshold {
            return None;
        }
        Some(window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_stream_no_output() {
        let mut pipeline = StreamingPipeline::new(16000);
        let silence = vec![0i16; 32000]; // 2s of silence
        let result = pipeline.push_frame(&silence);
        assert!(result.is_none());
    }

    #[test]
    fn loud_stream_produces_window() {
        let mut pipeline = StreamingPipeline::new(16000);
        let loud = vec![16000i16; 80000]; // 5s of audio, 3s window → slides
        let result = pipeline.push_frame(&loud);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 48000); // window_samples = 16000 * 3
    }

    #[test]
    fn sliding_window_advances_and_keeps_overlap() {
        // window 3s / step 1s at 16k. After a window fires, the buffer must
        // hold the tail from step_samples onward (2s overlap), not the stale
        // head - otherwise windows stop advancing past the first 2s.
        let mut pipeline = StreamingPipeline::new(16000);
        let mut audio = vec![0i16; 64000]; // 4s
        for (i, s) in audio.iter_mut().enumerate() {
            *s = (i % 30000) as i16;
        }
        pipeline.push_frame(&audio[..32000]); // 2s, no window yet
        let w1 = pipeline.push_frame(&audio[32000..48000]).unwrap();
        assert_eq!(w1, audio[..48000]);
        assert_eq!(pipeline.buffer_len(), 32000); // kept 2s of overlap
        let w2 = pipeline.push_frame(&audio[48000..]).unwrap();
        assert_eq!(w2, audio[16000..]); // contiguous advance: [1s..4s]
    }

    #[test]
    fn overlap_merge_dedupes() {
        let mut pipeline = StreamingPipeline::new(16000);
        let r1 = pipeline.merge_overlap("hello world this is");
        assert_eq!(r1, "hello world this is");
        let r2 = pipeline.merge_overlap("this is a test now");
        assert_eq!(r2, "a test now");
    }

    #[test]
    fn overlap_merge_no_overlap() {
        let mut pipeline = StreamingPipeline::new(16000);
        pipeline.merge_overlap("hello world");
        let r2 = pipeline.merge_overlap("completely different");
        assert_eq!(r2, "completely different");
    }
}
