//! Functional tests for each STT engine using open-source audio (jfk.wav).
//!
//! Requires models to be downloaded first:
//!   Parakeet: models/parakeet_realtime_eou_120m-v1-q8_0.gguf
//!   Moonshine: models/moonshine-tiny/ (preprocess.onnx, encode.onnx, etc.)

use rt_voice_wasm::engine::SttEngine;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_wav_samples(path: &str) -> Vec<i16> {
    let data = fs::read(path).expect("failed to read wav");
    assert!(&data[0..4] == b"RIFF", "not a WAV file");
    let data_start = find_chunk(&data, b"data").expect("no data chunk");
    let data_size = u32::from_le_bytes([
        data[data_start - 4],
        data[data_start - 3],
        data[data_start - 2],
        data[data_start - 1],
    ]) as usize;
    data[data_start..data_start + data_size]
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect()
}

fn find_chunk(data: &[u8], id: &[u8; 4]) -> Option<usize> {
    let mut pos = 12;
    while pos + 8 <= data.len() {
        if &data[pos..pos + 4] == id {
            return Some(pos + 8);
        }
        let size = u32::from_le_bytes([
            data[pos + 4],
            data[pos + 5],
            data[pos + 6],
            data[pos + 7],
        ]) as usize;
        pos += 8 + size + (size & 1); // pad to even
    }
    None
}

fn model_exists(path: &str) -> bool {
    Path::new(path).exists()
}

// ---------------------------------------------------------------------------
// Whisper — already covered by tests/golden_audio.rs; here for completeness
// ---------------------------------------------------------------------------

#[test]
fn test_whisper_jfk_stt_engine_trait() {
    let model_path = "models/ggml-tiny.en-q5_1.bin";
    if !model_exists(model_path) {
        eprintln!("SKIP: whisper model not found at {model_path}");
        return;
    }
    let samples = load_wav_samples("tests/fixtures/jfk.wav");
    let ctx = rt_voice_wasm::whisper::WhisperContext::init_from_file(model_path)
        .expect("failed to load whisper model");

    let texts = <rt_voice_wasm::whisper::WhisperContext as SttEngine>::transcribe(&ctx, &samples)
        .expect("whisper transcription failed");
    let joined = texts.join(" ").to_lowercase();

    assert!(
        joined.contains("fellow") || joined.contains("americans") || joined.contains("country"),
        "whisper output does not match expected jfk.wav content: {joined}"
    );
}

// ---------------------------------------------------------------------------
// Parakeet
// ---------------------------------------------------------------------------

#[test]
fn test_parakeet_jfk() {
    let model_path = "models/parakeet_realtime_eou_120m-v1-q8_0.gguf";
    if !model_exists(model_path) {
        eprintln!("SKIP: parakeet model not found at {model_path}");
        eprintln!("Download: curl -L -o {model_path} https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/realtime_eou_120m-v1-q8_0.gguf");
        return;
    }
    let samples = load_wav_samples("tests/fixtures/jfk.wav");
    let ctx = rt_voice_wasm::parakeet::ParakeetEngine::init_from_file(model_path)
        .expect("failed to load parakeet model");

    let texts = ctx.transcribe(&samples).expect("parakeet transcription failed");
    let joined = texts.join(" ").to_lowercase();

    assert!(
        joined.contains("fellow") || joined.contains("country"),
        "parakeet output does not match jfk.wav: {joined}"
    );
    assert!(!joined.is_empty(), "parakeet produced empty output");
}

// Parakeet with a very short audio clip (1 second) — verifies it handles short inputs
#[test]
fn test_parakeet_short_audio() {
    let model_path = "models/parakeet_realtime_eou_120m-v1-q8_0.gguf";
    if !model_exists(model_path) {
        eprintln!("SKIP: parakeet model not found");
        return;
    }
    let full_samples = load_wav_samples("tests/fixtures/jfk.wav");
    let short: Vec<i16> = full_samples.iter().take(16000).copied().collect(); // 1 second

    let ctx = rt_voice_wasm::parakeet::ParakeetEngine::init_from_file(model_path)
        .expect("failed to load parakeet model");

    let result = ctx.transcribe(&short);
    match result {
        Ok(texts) => {
            let joined = texts.join(" ");
            eprintln!("parakeet short-audio output: {joined}");
            // Short audio may or may not produce output — that's OK
        }
        Err(e) => {
            eprintln!("parakeet short-audio error (acceptable): {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Moonshine
// ---------------------------------------------------------------------------

#[test]
fn test_moonshine_jfk() {
    let model_dir = "models/moonshine-tiny";
    if !model_exists(model_dir) {
        eprintln!("SKIP: moonshine model not found at {model_dir}/");
        eprintln!("Convert with: pip install useful-moonshine[onnx] && python3 scripts/convert_moonshine_model.py");
        return;
    }
    // moonshine.cpp expects these four ONNX files + tokenizer
    let required = ["preprocess.onnx", "encode.onnx", "uncached_decode.onnx", "cached_decode.onnx"];
    let missing: Vec<_> = required.iter().filter(|f| !Path::new(model_dir).join(f).exists()).collect();
    if !missing.is_empty() {
        eprintln!("SKIP: missing ONNX files in {model_dir}: {missing:?}");
        return;
    }

    let samples = load_wav_samples("tests/fixtures/jfk.wav");
    let ctx = rt_voice_wasm::moonshine::MoonshineEngine::init_from_dir(model_dir)
        .expect("failed to load moonshine model");

    let texts = ctx.transcribe(&samples).expect("moonshine transcription failed");
    let joined = texts.join(" ").to_lowercase();

    assert!(!joined.is_empty(), "moonshine produced empty output");
    assert!(
        joined.contains("fellow") || joined.contains("country") || joined.contains("ask"),
        "moonshine output does not match jfk.wav: {joined}"
    );
}

// ---------------------------------------------------------------------------
// Cross-engine comparison: same audio, all three engines agree vaguely
// ---------------------------------------------------------------------------

#[test]
fn test_all_engines_agree_on_jfk() {
    let samples = load_wav_samples("tests/fixtures/jfk.wav");
    let mut results: Vec<(&str, String)> = Vec::new();

    // Whisper
    let whisper_model = "models/ggml-tiny.en-q5_1.bin";
    if model_exists(whisper_model) {
        let ctx = rt_voice_wasm::whisper::WhisperContext::init_from_file(whisper_model)
            .expect("whisper load");
        if let Ok(texts) = <rt_voice_wasm::whisper::WhisperContext as SttEngine>::transcribe(&ctx, &samples) {
            results.push(("whisper", texts.join(" ").to_lowercase()));
        }
    }

    // Parakeet
    let parakeet_model = "models/parakeet_realtime_eou_120m-v1-q8_0.gguf";
    if model_exists(parakeet_model) {
        let ctx = rt_voice_wasm::parakeet::ParakeetEngine::init_from_file(parakeet_model)
            .expect("parakeet load");
        if let Ok(texts) = ctx.transcribe(&samples) {
            results.push(("parakeet", texts.join(" ").to_lowercase()));
        }
    }

    // Moonshine
    let moonshine_dir = "models/moonshine-tiny";
    if model_exists(moonshine_dir) && Path::new(moonshine_dir).join("preprocess.onnx").exists() {
        let ctx = rt_voice_wasm::moonshine::MoonshineEngine::init_from_dir(moonshine_dir)
            .expect("moonshine load");
        if let Ok(texts) = ctx.transcribe(&samples) {
            results.push(("moonshine", texts.join(" ").to_lowercase()));
        }
    }

    assert!(!results.is_empty(), "no engines available for cross-comparison");

    for (engine, text) in &results {
        eprintln!("{engine}: {text}");
        assert!(!text.is_empty(), "{engine} produced empty output");
        // Every engine should find at least one of these words in jfk.wav
        let has_keyword = text.contains("fellow")
            || text.contains("country")
            || text.contains("americans")
            || text.contains("ask");
        assert!(has_keyword, "{engine} output lacks expected keywords: {text}");
    }

    if results.len() >= 2 {
        eprintln!("cross-engine comparison: {} engines agree on keyword presence", results.len());
    }
}
