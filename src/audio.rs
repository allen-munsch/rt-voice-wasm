pub struct PcmRing {
    buf: Vec<i16>,
    pos: usize,
    capacity: usize,
}

impl PcmRing {
    pub fn new(capacity_samples: usize) -> Self {
        PcmRing {
            buf: vec![0; capacity_samples],
            pos: 0,
            capacity: capacity_samples,
        }
    }

    pub fn push(&mut self, samples: &[i16]) {
        for &s in samples {
            self.buf[self.pos % self.capacity] = s;
            self.pos += 1;
        }
    }

    pub fn filled(&self) -> bool {
        self.pos >= self.capacity
    }

    pub fn snapshot(&self) -> Vec<i16> {
        if self.pos < self.capacity {
            Vec::from(&self.buf[..self.pos])
        } else {
            let start = self.pos % self.capacity;
            let mut out = Vec::with_capacity(self.capacity);
            out.extend_from_slice(&self.buf[start..]);
            out.extend_from_slice(&self.buf[..start]);
            out
        }
    }
}

pub fn downsample_to_16k(samples: &[f32], input_rate: u32) -> Vec<i16> {
    if input_rate == 16000 {
        return samples
            .iter()
            .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
            .collect();
    }
    let ratio = input_rate as f64 / 16000.0;
    let out_len = (samples.len() as f64 / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx = (i as f64 * ratio).round() as usize;
        let s = samples.get(src_idx).copied().unwrap_or(0.0);
        out.push((s * 32767.0).clamp(-32768.0, 32767.0) as i16);
    }
    out
}

/// Decode G.711 μ-law byte to 16-bit linear PCM.
pub fn mulaw_decode(mulaw: u8) -> i16 {
    const BIAS: u32 = 33;
    let data = (!mulaw) as u32;
    let sign = if data & 0x80 != 0 { -1i32 } else { 1 };
    let data = data & 0x7F;
    let exponent = (data >> 4) + 5;
    let mantissa = data & 0x0F;
    let decoded = ((1 << exponent)
        | (mantissa << (exponent - 4))
        | (1 << (exponent - 5)))
        .wrapping_sub(BIAS);
    (sign * decoded as i32) as i16
}

/// Decode a buffer of μ-law bytes to 16-bit PCM, then upsample from 8kHz to 16kHz.
pub fn decode_mulaw_8k_to_16k(mulaw: &[u8]) -> Vec<i16> {
    let pcm_8k: Vec<i16> = mulaw.iter().map(|&b| mulaw_decode(b)).collect();
    let mut out = Vec::with_capacity(pcm_8k.len() * 2);
    if pcm_8k.is_empty() {
        return out;
    }
    for i in 0..pcm_8k.len() - 1 {
        out.push(pcm_8k[i]);
        let mid = (pcm_8k[i] as i32 + pcm_8k[i + 1] as i32) / 2;
        out.push(mid as i16);
    }
    out.push(*pcm_8k.last().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulaw_silence() {
        // μ-law 0xFF is zero (silence)
        assert_eq!(mulaw_decode(0xFF), 0);
    }

    #[test]
    fn mulaw_roundtrip() {
        // μ-law: 0x80 is max positive (~8031), 0x00 is max negative (~-8031)
        let pos_max = mulaw_decode(0x80);
        let neg_max = mulaw_decode(0x00);
        assert!(pos_max > 8000, "pos_max={pos_max}");
        assert!(neg_max < -8000, "neg_max={neg_max}");
    }

    #[test]
    fn decode_mulaw_upsample_length() {
        let input = vec![0xFFu8; 800]; // 100ms of silence at 8kHz
        let out = decode_mulaw_8k_to_16k(&input);
        assert_eq!(out.len(), 1599);
        assert!(out.iter().all(|&s| s == 0));
    }

    #[test]
    fn ring_basic() {
        let mut ring = PcmRing::new(4);
        assert!(!ring.filled());
        ring.push(&[1, 2, 3]);
        assert!(!ring.filled());
        ring.push(&[4]);
        assert!(ring.filled());
        assert_eq!(ring.snapshot(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn ring_wraps() {
        let mut ring = PcmRing::new(4);
        ring.push(&[1, 2, 3, 4, 5]);
        assert_eq!(ring.snapshot(), vec![2, 3, 4, 5]);
    }

    #[test]
    fn downsample_passthrough() {
        let input: Vec<f32> = (0..160).map(|i| (i as f32 - 80.0) / 32768.0).collect();
        let out = downsample_to_16k(&input, 16000);
        assert_eq!(out.len(), 160);
    }

    #[test]
    fn downsample_48k_to_16k() {
        let input: Vec<f32> = vec![0.5; 48000];
        let out = downsample_to_16k(&input, 48000);
        assert_eq!(out.len(), 16000);
    }
}
