use crate::stream::StreamingPipeline;
use std::collections::VecDeque;
use std::sync::Mutex;
use wasm_bindgen::prelude::*;

static PIPELINE: Mutex<Option<StreamingPipeline>> = Mutex::new(None);
static OUTPUT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

#[wasm_bindgen]
pub fn init_pipeline(sample_rate: u32) {
    let mut pipe = PIPELINE.lock().unwrap();
    *pipe = Some(StreamingPipeline::new(sample_rate));
}

#[wasm_bindgen]
pub fn push_audio(samples: &[i16]) -> Option<String> {
    let mut pipe = PIPELINE.lock().unwrap();
    let pipeline = pipe.as_mut().expect("init_pipeline not called");

    if let Some(window) = pipeline.push_frame(samples) {
        // In the real impl, window goes to whisper inference.
        let text = format!("[window: {} samples]", window.len());
        let merged = pipeline.merge_overlap(&text);
        if !merged.is_empty() {
            let mut out = OUTPUT.lock().unwrap();
            out.push_back(merged);
        }
        return Some(text);
    }
    None
}

#[wasm_bindgen]
pub fn flush() -> String {
    let mut out = OUTPUT.lock().unwrap();
    let texts: Vec<String> = out.drain(..).collect();
    texts.join(" ")
}

#[wasm_bindgen]
pub fn reset() {
    let mut pipe = PIPELINE.lock().unwrap();
    *pipe = None;
    let mut out = OUTPUT.lock().unwrap();
    out.clear();
}
