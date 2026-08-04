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

#[cfg(test)]
mod tests {
    use super::*;

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
