---
name: rt-voice-wasm-debian-port
description: Package the rt-voice-wasm Rust crate (whisper.cpp bindings) as a Debian accessibility desktop app - libstdc++ auto-scan in build.rs, native-cpu/mic-capture cargo features, cpal mic capture (src/capture.rs), read_wav_i16 headless e2e, cargo-deb metadata. Verified on rt-voice-wasm (Debian 12/Ubuntu 24.04, whisper.cpp 1.9.1).
---

# Debian desktop port (live-captions + .deb)

## Feature flags (Cargo.toml)
- `native-cpu` (default): gates `-march=native` in build.rs so packaged builds stay portable.
- `mic-capture`: cpal is `optional = true` behind this feature. `live-captions` bin sets `required-features = ["mic-capture"]`, so the lib and the server/ws-send bins compile without ALSA; only the mic path needs it.

## build.rs gotchas
- Do NOT hardcode `/usr/lib/gcc/x86_64-linux-gnu/13`. Scan the dir for the highest version containing `libstdc++.a` (Debian 12 has gcc-12, Ubuntu 22.04 gcc-11, newer gcc-14+).
- Link `cargo:rustc-link-lib=static=stdc++`; being static means the .deb needs no libstdc++ runtime dep.
- `-march=native` only when the `native-cpu` feature is on (checked via `env::var("CARGO_FEATURE_NATIVE_CPU").is_ok()`).

## New files
- `src/capture.rs`: `MicCapture` via cpal — stereo->mono downmix, then downsample to 16k reusing `audio::downsample_to_16k`.
- `src/audio.rs`: `read_wav_i16(path) -> Result<(Vec<i16>, u32)>` for `--wav-file` headless e2e; rejects non-PCM and non-16-bit files.
- `src/bin/live_captions.rs`: flags `--model --wav-file --device --speed --agent-hook --list-devices --use-moonshine`. Default model: `/usr/share/rt-voice-wasm/models/ggml-tiny.en-q5_1.bin`, fallback `./models/`. Emits JSON-lines stdout `{"event":"transcript","text":...}` plus `latency` events.

## cargo-deb
```toml
[package.metadata.deb]
maintainer = "James Munsch <james.a.munsch@gmail.com>"
section = "sound"
priority = "optional"
depends = "libasound2 | libasound2t64"
assets = [
    ["models/ggml-tiny.en-q5_1.bin","/usr/share/rt-voice-wasm/models/","644"],
    ["target/release/live-captions","/usr/bin/live-captions","755"],
]
```
- Build with `cargo deb --no-default-features --features mic-capture` — deb must NOT use `-march=native`.
- Verify contents with `dpkg -c target/debian/rt-voice-wasm_0.1.0-1_amd64.deb` (expect /usr/bin/live-captions + the model under /usr/share).

## Verification
- `cargo test`: 53 lib tests (default), 56 (with mic-capture), integration all pass.
- e2e: `cargo run --bin live-captions --features mic-capture -- --wav-file tests/fixtures/jfk.wav` — expect full transcript and RTF 0.20-0.43 (well under real-time).
- Moonshine engine tests need `LD_LIBRARY_PATH=build/moonshine` (dlopen via libloading; direct linking crashes with `free(): invalid pointer`).
