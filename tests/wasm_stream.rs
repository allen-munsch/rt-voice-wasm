#![cfg(target_arch = "wasm32")]

use rt_voice_wasm::wasm;
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
fn init_and_push() {
    wasm::init_pipeline(16000);
    let samples = vec![16000i16; 64000]; // 4s of loud audio
    let result = wasm::push_audio(&samples);
    assert!(result.is_some());
    let text = result.unwrap();
    assert!(text.contains("window"));
}

#[wasm_bindgen_test]
fn silent_audio_no_output() {
    wasm::init_pipeline(16000);
    let silence = vec![0i16; 64000];
    let result = wasm::push_audio(&silence);
    assert!(result.is_none());
}

#[wasm_bindgen_test]
fn flush_collects_output() {
    wasm::init_pipeline(16000);
    let loud = vec![16000i16; 64000];
    wasm::push_audio(&loud);
    wasm::push_audio(&loud);
    let output = wasm::flush();
    assert!(!output.is_empty());
}

#[wasm_bindgen_test]
fn reset_clears_state() {
    wasm::init_pipeline(16000);
    let loud = vec![16000i16; 64000];
    wasm::push_audio(&loud);
    wasm::reset();
    wasm::init_pipeline(16000);
    let result = wasm::push_audio(&[0i16; 100]);
    assert!(result.is_none());
}
