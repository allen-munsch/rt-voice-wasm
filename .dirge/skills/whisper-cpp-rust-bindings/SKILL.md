---
name: whisper-cpp-rust-bindings
description: Build whisper.cpp into a Rust crate using cc + bindgen, link static libstdc++, and split native/wasm32 bindings. Verified on rt-voice-wasm (whisper.cpp 1.9.1, Ubuntu x86_64).
---

# Building whisper.cpp into Rust (cc + bindgen)

## Vendor

- Check out whisper.cpp into `third_party/whisper.cpp` (submodule or tarball).
- Do not build via its own Makefile/CMake; compile the C/C++ files directly with the `cc` crate.

## build.rs recipe

- C files (ggml `src/*.c`) and C++ files (`src/whisper.cpp`, `ggml/src/ggml-backend-reg.cpp`) both compiled via `cc::Build`.
- Required defines on the C build or headers fail:
  - `_GNU_SOURCE`
  - `WHISPER_VERSION="1.9.1"` and `GGML_VERSION="1.9.1"`
  - `GGML_COMMIT="unknown"`
  - `GGML_SCHED_MAX_COPIES=2` (start at 4; 2 measured faster on i9-13900H)
- Flags: `-std=c11`, `-pthread`, `-I` for `include/`, `ggml/include/`, `src/`. `warnings(false)`.
- Host perf flags on BOTH the C and C++ builds: `-march=native -ffast-math -fno-finite-math-only`. `-ffast-math` alone produces wrong/NaN results in ggml CPU ops — keep `-fno-finite-math-only` after it.
- When packaging for distros (.deb), gate `-march=native` behind a `native-cpu` Cargo feature (default-on) so `--no-default-features` yields a binary that runs on older CPUs. `-ffast-math`/`-fno-finite-math-only` can stay unconditional — verified both `cargo build` and `cargo build --no-default-features` succeed with the gate.
- `println!("cargo:rerun-if-changed=third_party/whisper.cpp/")`.

## Linking libstdc++ (the only incantation that worked)

```rust
// scan, don't hardcode the gcc version: Debian 12 ships gcc-12, Ubuntu 22.04 gcc-11,
// so a hardcoded "/13" breaks the build on both (verified fix in rt-voice-wasm).
// Glob /usr/lib/gcc/x86_64-linux-gnu/*/libstdc++.a, take the highest version dir.
let ver = highest_version_dir("/usr/lib/gcc/x86_64-linux-gnu"); // e.g. "13"
println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/{ver}");
println!("cargo:rustc-link-lib=static=stdc++");
```

- `dylib=stdc++` and search path `/usr/lib/x86_64-linux-gnu` FAIL: that dir only has `libstdc++.so.6` (no unversioned `.so` dev symlink). The static archive lives under the gcc dir. Check with `ls /usr/lib/gcc/x86_64-linux-gnu/*/libstdc++.a`.

## bindgen

- Header: `whisper.h` with `-I` for `include/` and `ggml/include/`.
- Allowlist (exact names that exist in 1.9.1):
  - `whisper_init_from_file_with_params` (NOT `..._no_state` in this version)
  - `whisper_free`, `whisper_full`, `whisper_full_n_segments`, `whisper_full_get_segment_text`, `whisper_full_get_segment_t0`, `whisper_full_get_segment_t1`
  - `whisper_full_default_params` (and `whisper_free_params`)
  - `whisper_context_default_params`
  - `whisper_print_timings`, `whisper_reset_timings` (for profiling)
  - types `whisper_context_params`, `whisper_full_params`

## API usage rules

- Init: `whisper_context_default_params()` then `whisper_init_from_file_with_params(path, params)`. Do NOT hand-write a `whisper_context_params` struct literal — fields churn between versions (`dtw_aheads_preset` → `whisper_alignment_heads_preset_WHISPER_AHEADS_NONE`, `dtw_mem_size` u64 → usize).
- Transcribe: `whisper_full_default_params(whisper_sampling_strategy_WHISPER_SAMPLING_GREEDY)` BY VALUE — never `_by_ref` (that returns a pointer to static/thread-local storage that must not be freed). No `whisper_free_params` call needed for the by-value path.
- Samples: i16 → f32 by `s / 32768.0`, n_samples as i32.
- `params.language = std::ptr::null()` for auto-detect; set `n_threads` (cap at 4), `print_progress/realtime/timestamps = false`.

## Native / wasm32 split

- `src/lib.rs`:
  ```rust
  #[cfg(not(target_arch = "wasm32"))]
  #[path = "whisper_host.rs"]
  pub mod whisper;
  #[cfg(target_arch = "wasm32")]
  #[path = "whisper_stub.rs"]
  pub mod whisper;
  ```
  Flat files `src/whisper_host.rs` / `src/whisper_stub.rs` — NOT a `src/whisper/` dir.
- wasm-bindgen export name collision: the generated JS glue already exports `init`, so a Rust `pub fn init` breaks the JS import. Rename to `init_pipeline`.
- wasm tests: `tests/wasm_stream.rs` with `wasm_bindgen_test_configure!(run_in_node)`.

## Browser inference path (official Emscripten build)

- Browser transcription does NOT use the wasm-bindgen whisper stub. `web/whisper/whisper.js` + `web/whisper/helpers.js` is the official whisper.cpp Emscripten module (`examples/whisper.wasm`).
- Build: `source /tmp/emsdk/emsdk_env.sh`, then `emcmake cmake ..` in `third_party/whisper.cpp/build-wasm`; copy `bin/whisper.wasm/main.js` → `web/whisper/whisper.js` (1.7 MB, wasm embedded).
- `third_party/whisper.cpp/examples/whisper.wasm/emscripten.cpp` was patched to add `#include <thread>` — the build fails without it.
- A custom `bridge.c` (raw `-s EXPORTED_FUNCTIONS` + ccall wrappers with index handles) was built and then ABANDONED in favor of the official emscripten.cpp binding API. Do not resurrect bridge.c.
- Dev server must send COOP/COEP headers (`Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`) for SharedArrayBuffer; plain `python3 -m http.server` is insufficient. Use the inline python `http.server` subclass that adds both headers.

## Perf tuning (host)

- `whisper_full_params` that matter: pin `language="en"`, `no_timestamps=true`, `single_segment=true`, `suppress_blank=true`, `suppress_nst=true`, `temperature=0.0`, `n_threads=8`. On i9-13900H with tiny.en-q5_1 this took RTF from ~5.0 to 0.023.
- Encoder (mel + 4 transformer layers) is ~60-70% of inference; decoder ~30-40%. With RTF < 0.03 the latency ceiling is the streaming window length, not compute.
- Also cap `params.duration_ms` to the actual audio length (default assumes full 30s context).

## Realtime streaming pipeline (runtime architecture)

- `StreamingPipeline` (src/stream.rs): window/step overlap loop with an RMS threshold gate; `with_speed(16000, factor)` shrinks the window for phone latency (2s window / 1s step in the server).
- `WhisperContext` has `unsafe impl Sync`, so a single context can be Arc-shared across `tokio::spawn` tasks.
- Phone path (src/bin/server.rs): Twilio Media Streams WebSocket listener on 8080. Base64 mu-law 8k payloads, decode with `decode_mulaw_8k_to_16k` (src/audio.rs), run `StreamingPipeline::with_speed(16000, factor)`, then `WhisperContext::transcribe`, then emit JSON events back over the WS (`transcript`, `agent_action`).
- src/agent.rs: zero-dependency intent router, keyword substring match resolves to `Action::{Respond, Transfer, Escalate, Hangup, Continue}`; server.rs drives a Greeting, Routing, Respond/Transfer/Escalate, Closing state machine.
- src/audio.rs extras: `mulaw_decode` (LUT) and `speedup()` (linear resampler; 1.5-2x cuts whisper wall-time).
- RawWsTransport wire format is {"event": <kind>, "text": <text>} - the browser client must parse `msg.event` (the Rust struct field is `kind`, the JSON key is `event`).
- Transport threading: the SplitSink lives in a dedicated std::thread doing `tokio::Handle::current().block_on()`; `Handle::current()` panics outside a tokio context. The writer thread pins the socket open until the std mpsc Sender drops, so final events (e.g. `full_transcript`) must be enqueued before the transport drops.

## Verification

`cargo build && cargo test --lib`

- End-to-end native: run the compiled `target/debug/deps/golden_audio-*` test binary feeding `jfk.wav` through a quantized model (e.g. `ggml-tiny.en-q5_1.bin`).
- Wasm: `wasm-pack build` → `pkg/` with `.d.ts`, `_bg.wasm`, `rt_voice_wasm.js`; `web/main.js` imports `init_pipeline, push_audio, flush, reset` from `../pkg/rt_voice_wasm.js`.
