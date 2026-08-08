---
name: mic-capture-cpal
description: Add desktop microphone capture to a Rust whisper.cpp crate with cpal on Debian/Ubuntu - optional-dep feature gating, ALSA dev requirement, stereo-to-mono downmix, WAV file input, live-caption binary. Verified on rt-voice-wasm (milestone 1).
---

# Mic capture with cpal in a whisper.cpp Rust crate

Verified on rt-voice-wasm (Debian accessibility port). Keeps the lib and existing binaries building on machines without ALSA.

## Cargo.toml
- `cpal = { version = "0.15", optional = true }` under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
- `[features] mic-capture = ["dep:cpal"]` (plus a `native-cpu = []` default feature if you gate `-march=native` in build.rs for portable packaging builds).
- The mic binary gets `required-features = ["mic-capture"]` so `cargo build --bins` and `cargo test` compile without ALSA.

## src/capture.rs
- `cpal::default_host()` -> `input_devices()` -> enumerate names; pick default input config.
- Convert device sample rate to 16k: if rate != 16000, run each channel through the crate's existing `downsample_to_16k` (e.g. `audio.rs`), else pass through.
- Downmix multi-channel to mono per frame: `(sum of channel samples) / channels as i32`.

## src/audio.rs
- `read_wav_i16(path) -> Result<(Vec<i16>, u32)>`: parse RIFF, find `fmt ` chunk (require PCM=1, 16-bit, else error), find `data` chunk, then frame-average to mono if channels >= 2. TDD with a hand-synthesized WAV written to /tmp (build RIFF header + chunks in the test itself).

## src/bin/live_captions.rs
- Hand-rolled flag parsing (match on args, like server.rs), `--list-devices` smoke flag, `--wav-file` for offline input, loop pushing 16k mono i16 into StreamingPipeline, emit `{"event":"transcript","text":...}` JSON.

## Pitfalls
- cpal won't link without `libasound2-dev` (Debian/Ubuntu). `pkg-config --exists alsa` to check. `sudo apt install libasound2-dev` fixes `cargo check --bins`.
- Do NOT make cpal a hard dep: machines without ALSA (CI, minimal containers) fail at build time, not runtime.
- `--no-default-features` must still build: it's the packaging mode (no `-march=native`).

## Verification
`cargo test --features mic-capture 2>&1 | tail -3 && cargo build --bins 2>&1 | tail -2 && cargo check --no-default-features 2>&1 | tail -2`
