//! Deterministic audio augmentation for adversarial STT testing.
//! Every function is pure: takes &[i16], returns Vec<i16>.

use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;

/// Insert a silent pause of `duration_ms` milliseconds at `position_samples`.
/// Position 0 = at start, position >= len = at end.
pub fn pause(samples: &[i16], position_samples: usize, duration_ms: u32, sample_rate: u32) -> Vec<i16> {
    let silence_len = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
    let pos = position_samples.min(samples.len());
    let mut out = Vec::with_capacity(samples.len() + silence_len);
    out.extend_from_slice(&samples[..pos]);
    out.resize(out.len() + silence_len, 0);
    out.extend_from_slice(&samples[pos..]);
    out
}

/// Stutter: repeat `repeat_len` samples starting at `position` for `count` repetitions.
pub fn stutter(samples: &[i16], position: usize, repeat_len: usize, count: usize) -> Vec<i16> {
    if position >= samples.len() || repeat_len == 0 || count == 0 {
        return samples.to_vec();
    }
    let end = (position + repeat_len).min(samples.len());
    let segment: Vec<i16> = samples[position..end].to_vec();
    let mut out = Vec::with_capacity(samples.len() + segment.len() * count);
    out.extend_from_slice(&samples[..position]);
    for _ in 0..count {
        out.extend_from_slice(&segment);
    }
    out.extend_from_slice(&samples[position..]);
    out
}

/// Reverse a segment from `start` to `end` (exclusive). Simulates a speaker
/// correcting themselves by restating something in reverse order.
pub fn reverse_segment(samples: &[i16], start: usize, end: usize) -> Vec<i16> {
    if start >= samples.len() || end <= start {
        return samples.to_vec();
    }
    let end = end.min(samples.len());
    let mut out = samples.to_vec();
    out[start..end].reverse();
    out
}

/// Randomly drop samples with probability `pct` (0.0–1.0). Deterministic via seed.
pub fn dropout(samples: &[i16], pct: f64, seed: u64) -> Vec<i16> {
    if pct <= 0.0 {
        return samples.to_vec();
    }
    let pct = pct.min(1.0);
    let mut rng = StdRng::seed_from_u64(seed);
    let keep = 1.0 - pct;
    samples.iter().filter(|_| rng.r#gen::<f64>() < keep).copied().collect()
}

/// Add white noise at a given SNR in dB. Higher SNR = quieter noise.
/// Deterministic via seed.
pub fn add_noise(samples: &[i16], snr_db: f64, seed: u64) -> Vec<i16> {
    if snr_db > 120.0 || samples.is_empty() {
        return samples.to_vec();
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let signal_rms: f64 = (samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    let noise_rms = signal_rms / 10f64.powf(snr_db / 20.0);
    samples
        .iter()
        .map(|&s| {
            let n: f64 = rng.r#gen_range(-1.0..1.0) * noise_rms;
            (s as f64 + n).clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

/// Concatenate two sample buffers — simulates overlapping speech or a speaker
/// resuming after a pause.
pub fn concat(a: &[i16], b: &[i16]) -> Vec<i16> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    out.extend_from_slice(a);
    out.extend_from_slice(b);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_at_start() {
        let samples = vec![100i16, 200, 300];
        let out = pause(&samples, 0, 100, 16000);
        assert_eq!(out.len(), samples.len() + 1600);
        assert!(out[..1600].iter().all(|&s| s == 0));
        assert_eq!(&out[1600..], &[100, 200, 300]);
    }

    #[test]
    fn pause_at_end() {
        let samples = vec![100i16, 200, 300];
        let out = pause(&samples, 3, 100, 16000);
        assert_eq!(out.len(), samples.len() + 1600);
        assert_eq!(&out[..3], &[100, 200, 300]);
        assert!(out[3..].iter().all(|&s| s == 0));
    }

    #[test]
    fn stutter_basic() {
        let samples = vec![1i16, 2, 3, 4, 5];
        let out = stutter(&samples, 1, 2, 2);
        // [1, then 2-rep of [2,3], then rest from pos 1 onward: 2,3,4,5]
        assert_eq!(out, vec![1, 2, 3, 2, 3, 2, 3, 4, 5]);
    }

    #[test]
    fn stutter_empty() {
        let samples = vec![1i16, 2, 3];
        let out = stutter(&samples, 0, 0, 5);
        assert_eq!(out, samples);
    }

    #[test]
    fn reverse_segment_middle() {
        let samples = vec![1i16, 2, 3, 4, 5];
        let out = reverse_segment(&samples, 1, 4);
        assert_eq!(out, vec![1, 4, 3, 2, 5]);
    }

    #[test]
    fn dropout_all() {
        let samples = vec![1i16, 2, 3];
        let out = dropout(&samples, 1.0, 42);
        assert!(out.is_empty());
    }

    #[test]
    fn dropout_none() {
        let samples = vec![1i16, 2, 3];
        let out = dropout(&samples, 0.0, 42);
        assert_eq!(out, samples);
    }

    #[test]
    fn dropout_deterministic() {
        let samples: Vec<i16> = (0..100).map(|i| i as i16).collect();
        let out1 = dropout(&samples, 0.3, 42);
        let out2 = dropout(&samples, 0.3, 42);
        assert_eq!(out1, out2);
        assert!(out1.len() < 100);
        assert!(out1.len() > 50); // ~70 expected
    }

    #[test]
    fn add_noise_high_snr_is_near_original() {
        let samples: Vec<i16> = vec![1000; 100];
        let out = add_noise(&samples, 60.0, 42);
        assert_eq!(out.len(), samples.len());
        for (&orig, &noisy) in samples.iter().zip(out.iter()) {
            assert!((orig - noisy).abs() < 50, "sample diff too large: {} -> {}", orig, noisy);
        }
    }

    #[test]
    fn add_noise_deterministic() {
        let samples: Vec<i16> = (0..50).map(|i| i * 100).collect();
        let out1 = add_noise(&samples, 20.0, 99);
        let out2 = add_noise(&samples, 20.0, 99);
        assert_eq!(out1, out2);
    }
}
