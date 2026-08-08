//! Adversarial dictation scenario tests.
//!
//! Loads jfk.wav, applies audio transforms (stutter, pause, reversal, noise,
//! dropout) from tests/fixtures/adversarial-dictation.json, transcribes each
//! augmented variant with a real whisper model, and checks baseline quality
//! (min words produced, no pipeline crashes).
//!
//! Skips gracefully when no model is available.

use rt_voice_wasm::audio::{read_wav_i16, speedup};
use rt_voice_wasm::augment;
use rt_voice_wasm::stream::StreamingPipeline;
use rt_voice_wasm::whisper::WhisperContext;

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    #[allow(dead_code)]
    description: String,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    name: String,
    description: String,
    tier: u32,
    turns: Vec<Turn>,
}

#[derive(Debug, Deserialize)]
struct Turn {
    #[serde(default)]
    augment: Option<AugmentSpec>,
    #[serde(default)]
    expect: Option<ExpectSpec>,
}

#[derive(Debug, Deserialize)]
struct AugmentSpec {
    kind: String,
    #[serde(default)]
    position_ms: u64,
    #[serde(default)]
    repeat_ms: u64,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    start_ms: u64,
    #[serde(default)]
    end_ms: u64,
    #[serde(default)]
    snr_db: f64,
    #[serde(default)]
    pct: f64,
    #[serde(default)]
    seed: u64,
}

#[derive(Debug, Deserialize)]
struct ExpectSpec {
    #[serde(default)]
    min_words: usize,
}

fn wav_path() -> PathBuf {
    PathBuf::from("tests/fixtures/jfk.wav")
}

fn model_path() -> Option<PathBuf> {
    let candidates = [
        "models/ggml-tiny.en-q5_1.bin",
        "/usr/share/rt-voice-wasm/models/ggml-tiny.en-q5_1.bin",
    ];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn apply_augment(samples: &[i16], spec: &AugmentSpec) -> Vec<i16> {
    let rate = 16000u32;
    match spec.kind.as_str() {
        "stutter" => {
            let pos = (spec.position_ms as usize * rate as usize / 1000).min(samples.len());
            let rep_len = (spec.repeat_ms as usize * rate as usize / 1000).max(1);
            augment::stutter(samples, pos, rep_len, spec.count.max(1))
        }
        "pause" => {
            let pos = (spec.position_ms as usize * rate as usize / 1000).min(samples.len());
            augment::pause(samples, pos, spec.duration_ms as u32, rate)
        }
        "reverse" => {
            let start = (spec.start_ms as usize * rate as usize / 1000).min(samples.len());
            let end = (spec.end_ms as usize * rate as usize / 1000).min(samples.len());
            augment::reverse_segment(samples, start, end)
        }
        "noise" => augment::add_noise(samples, spec.snr_db, spec.seed),
        "dropout" => augment::dropout(samples, spec.pct, spec.seed),
        _ => samples.to_vec(),
    }
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

#[test]
fn adversarial_dictation_scenarios() {
    let model = match model_path() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: no model found; run scripts/download_model.sh first");
            return;
        }
    };

    let scenario_file = PathBuf::from("tests/fixtures/adversarial-dictation.json");
    let data = std::fs::read_to_string(&scenario_file)
        .expect("failed to read adversarial-dictation.json");
    let sf: ScenarioFile = serde_json::from_str(&data)
        .expect("failed to parse adversarial-dictation.json");

    // Load WAV once
    let (wav_samples, _rate) = read_wav_i16(wav_path().to_str().unwrap())
        .expect("failed to read jfk.wav");
    let ctx = WhisperContext::init_from_file(model.to_str().unwrap())
        .expect("failed to init whisper model");

    for scenario in &sf.scenarios {
        eprintln!("--- scenario: {} ---", scenario.name);

        // Each augment turn produces a fresh augmented copy; sequential turns
        // chain the augments on the same buffer.
        let mut current = wav_samples.clone();

        for turn in &scenario.turns {
            // Apply augment if specified
            if let Some(ref aug) = turn.augment {
                current = apply_augment(&current, aug);
                eprintln!("  augmented: {} -> {} samples", aug.kind, current.len());
            }

            // Transcribe
            let mut pipeline = StreamingPipeline::with_speed(16000, 1.0);
            let chunk_size = (16000 / 16).max(480);
            let mut all_text = String::new();

            for chunk in current.chunks(chunk_size) {
                if let Some(window) = pipeline.push_frame(chunk) {
                    let sped = speedup(&window, pipeline.speed_factor());
                    match ctx.transcribe(&sped) {
                        Ok((segments, _timing)) => {
                            let texts: Vec<String> = segments.iter().map(|s| s.text.clone()).collect();
                            let text: String = texts.join(" ");
                            let merged = pipeline.merge_overlap(&text);
                            if !merged.is_empty() {
                                all_text.push_str(&merged);
                                all_text.push(' ');
                            }
                        }
                        Err(e) => {
                            panic!("transcription error in '{}': {e}", scenario.name);
                        }
                    }
                }
            }
            // Flush remaining buffer
            // (StreamingPipeline doesn't have an explicit flush; last window is
            // emitted when buffer exceeds window_samples.)

            // Check expectations
            if let Some(ref expect) = turn.expect {
                let wc = word_count(&all_text);
                eprintln!("  transcript ({} words): '{}'", wc, &all_text[..all_text.len().min(100)]);
                assert!(
                    wc >= expect.min_words,
                    "[{}] expected >= {} words, got {}: '{}'",
                    scenario.name, expect.min_words, wc, all_text
                );
            }
        }
    }
}
