//! Native audio capture from microphone via cpal.
//! Stereo → mono downmix and sample-rate conversion to 16 kHz.

use crate::audio::downsample_to_16k;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct MicCapture {
    stream: cpal::Stream,
    rx: std::sync::mpsc::Receiver<Vec<i16>>,
}

impl MicCapture {
    /// Start capturing from the default input device at 16 kHz mono.
    /// Returns immediately; audio chunks arrive via `recv()`.
    pub fn start() -> Result<Self, String> {
        Self::start_with_device(None)
    }

    /// Start capturing from the named device (or default if None).
    pub fn start_with_device(device_name: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if let Some(name) = device_name {
            host.input_devices()
                .map_err(|e| format!("cpal input devices: {e}"))?
                .find(|d| d.name().map(|n| n == name).unwrap_or(false))
                .ok_or_else(|| format!("no input device named '{name}'"))?
        } else {
            host.default_input_device()
                .ok_or_else(|| "no default input device".to_string())?
        };

        let config = device
            .default_input_config()
            .map_err(|e| format!("default input config: {e}"))?;
        let input_rate = config.sample_rate().0;

        let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();

        let err_fn = |err| eprintln!("cpal stream error: {err}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &cpal::StreamConfig {
                        channels: config.channels(),
                        sample_rate: config.sample_rate(),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mono = downmix_f32(data, config.channels() as usize);
                        let i16 = downsample_to_16k(&mono, input_rate);
                        let _ = tx.send(i16);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("build f32 input stream: {e}")),
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &cpal::StreamConfig {
                        channels: config.channels(),
                        sample_rate: config.sample_rate(),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mono = downmix_i16(data, config.channels() as usize);
                        if input_rate == 16000 {
                            let _ = tx.send(mono);
                        } else {
                            let f32: Vec<f32> = mono.iter().map(|&s| s as f32 / 32768.0).collect();
                            let i16 = downsample_to_16k(&f32, input_rate);
                            let _ = tx.send(i16);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("build i16 input stream: {e}")),
            cpal::SampleFormat::U16 => device
                .build_input_stream(
                    &cpal::StreamConfig {
                        channels: config.channels(),
                        sample_rate: config.sample_rate(),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let f32: Vec<f32> =
                            data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect();
                        let mono = downmix_f32(&f32, config.channels() as usize);
                        let i16 = downsample_to_16k(&mono, input_rate);
                        let _ = tx.send(i16);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("build u16 input stream: {e}")),
            _ => return Err(format!("unsupported sample format: {:?}", config.sample_format())),
        }?;

        stream.play().map_err(|e| format!("play stream: {e}"))?;

        Ok(MicCapture { stream, rx })
    }

    /// Receive the next chunk of 16 kHz mono i16 samples.
    pub fn recv(&self) -> Option<Vec<i16>> {
        self.rx.recv().ok()
    }

    /// Non-blocking receive.
    pub fn try_recv(&self) -> Option<Vec<i16>> {
        self.rx.try_recv().ok()
    }

    /// List available input devices.
    pub fn list_devices() -> Result<Vec<String>, String> {
        let host = cpal::default_host();
        let devices = host
            .input_devices()
            .map_err(|e| format!("cpal input devices: {e}"))?;
        let names: Vec<String> = devices
            .filter_map(|d| d.name().ok())
            .collect();
        Ok(names)
    }
}

/// Downmix interleaved multi-channel f32 to mono (average).
fn downmix_f32(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let num_frames = samples.len() / channels;
    let mut out = Vec::with_capacity(num_frames);
    for frame in 0..num_frames {
        let sum: f32 = (0..channels).map(|c| samples[frame * channels + c]).sum();
        out.push(sum / channels as f32);
    }
    out
}

/// Downmix interleaved multi-channel i16 to mono (average).
fn downmix_i16(samples: &[i16], channels: usize) -> Vec<i16> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let num_frames = samples.len() / channels;
    let mut out = Vec::with_capacity(num_frames);
    for frame in 0..num_frames {
        let sum: i32 = (0..channels).map(|c| samples[frame * channels + c] as i32).sum();
        out.push((sum / channels as i32) as i16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_stereo_to_mono_f32() {
        let stereo: Vec<f32> = vec![1.0, 0.0, 0.5, 0.5, -1.0, -1.0];
        let mono = downmix_f32(&stereo, 2);
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.5).abs() < 0.001);
        assert!((mono[1] - 0.5).abs() < 0.001);
        assert!((mono[2] + 1.0).abs() < 0.001);
    }

    #[test]
    fn downmix_mono_passthrough_f32() {
        let mono_in: Vec<f32> = vec![1.0, 2.0, 3.0];
        let mono_out = downmix_f32(&mono_in, 1);
        assert_eq!(mono_out, mono_in);
    }

    #[test]
    fn downmix_stereo_to_mono_i16() {
        let stereo: Vec<i16> = vec![1000, 0, 500, 500, -1000, -1000];
        let mono = downmix_i16(&stereo, 2);
        assert_eq!(mono.len(), 3);
        assert_eq!(mono[0], 500);
        assert_eq!(mono[1], 500);
        assert_eq!(mono[2], -1000);
    }
}
