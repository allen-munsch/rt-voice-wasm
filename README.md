# rt-voice-wasm

Real-time audio transcription in the browser using
[whisper.cpp](https://github.com/ggerganov/whisper.cpp) compiled to WebAssembly,
with a Rust streaming pipeline.

## How it works

- Mic audio captured via Web Audio API / AudioWorklet at 16 kHz mono
- 4-second sliding windows with 2-second overlap, RMS silence gate
- Inference via official whisper.wasm (Emscripten, single-threaded SIMD)
- Transcript emitted live to the page

## Quick start

```bash
# Prerequisites
rustup target add wasm32-unknown-unknown
cargo install wasm-pack

# Download the model and test fixture
./scripts/download_model.sh
./scripts/fetch_fixture.sh

# Build and test (native)
cargo build
cargo test --lib
cargo test --release --test golden_audio

# Build WASM and serve
./scripts/build_wasm_cmake.sh
cp third_party/whisper.cpp/build-wasm/bin/whisper.wasm.js web/whisper/whisper.js
python3 web/serve.py
```

Open `http://localhost:8000/index.html` in Chrome or Edge.

## Wasm-bindgen tests

```bash
wasm-pack test --node
```

## Model

Uses `ggml-tiny.en-q5_1.bin` (~31 MB, quantized). Downloaded on first run from
HuggingFace by `scripts/download_model.sh`. The web harness fetches it at
startup with a progress bar.

## Architecture

| Layer | File |
|-------|------|
| Audio ring buffer & downsampler | `src/audio.rs` |
| Streaming pipeline (windows, VAD, overlap merge) | `src/stream.rs` |
| whisper.cpp FFI (native) | `src/whisper_host.rs` |
| whisper.cpp FFI (wasm32 stub) | `src/whisper_stub.rs` |
| wasm-bindgen exports | `src/wasm.rs` |
| Module routing | `src/lib.rs` |
| Build (cc + bindgen) | `build.rs` |
| Web harness | `web/` |

## Requirements

- Rust 1.80+
- Emscripten SDK (for whisper.wasm build)
- Python 3.8+ (for dev server)
