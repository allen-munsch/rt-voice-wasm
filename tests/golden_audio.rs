use rt_voice_wasm::whisper::WhisperContext;
use std::fs;

fn load_wav_samples(path: &str) -> Vec<i16> {
    let data = fs::read(path).expect("failed to read wav");
    assert!(&data[0..4] == b"RIFF", "not a WAV file");
    // Assume 16-bit PCM mono; find data chunk
    let data_start = find_chunk(&data, b"data").expect("no data chunk");
    let data_size = u32::from_le_bytes([
        data[data_start - 4],
        data[data_start - 3],
        data[data_start - 2],
        data[data_start - 1],
    ]) as usize;
    let samples: Vec<i16> = data[data_start..data_start + data_size]
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    samples
}

fn find_chunk(data: &[u8], id: &[u8; 4]) -> Option<usize> {
    let mut offset = 12;
    while offset + 8 <= data.len() {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;
        if chunk_id == id {
            return Some(offset + 8);
        }
        offset += 8 + chunk_size + (chunk_size & 1);
    }
    None
}

#[test]
fn golden_jfk_contains_expected_text() {
    let model_path = "models/ggml-tiny.en-q5_1.bin";
    let wav_path = "tests/fixtures/jfk.wav";

    if !fs::metadata(model_path).is_ok() {
        eprintln!("Skipping test: model not found at {model_path}. Run scripts/download_model.sh");
        return;
    }
    if !fs::metadata(wav_path).is_ok() {
        eprintln!("Skipping test: fixture not found at {wav_path}. Run scripts/fetch_fixture.sh");
        return;
    }

    let ctx = WhisperContext::init_from_file(model_path).expect("failed to load model");
    let samples = load_wav_samples(wav_path);

    let (segments, timing) = ctx.transcribe(&samples).expect("transcription failed");

    let full_text: String = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
    eprintln!("Transcript: {full_text}");
    eprintln!("Timing: total={:.0}ms encoder={:.0}ms decoder={:.0}ms",
        timing.total_ms, timing.encoder_ms, timing.decoder_ms);

    assert!(
        full_text.to_lowercase().contains("fellow americans"),
        "expected 'fellow Americans' in transcript, got: {full_text}"
    );

    let audio_duration = samples.len() as f64 / 16000.0;
    let rtf = timing.total_ms / 1000.0 / audio_duration;
    eprintln!("RTF: {rtf:.3} (audio {audio_duration:.1}s, inference {:.0}ms)", timing.total_ms);

    assert!(rtf < 5.0, "RTF {rtf:.2} exceeds threshold");
}
