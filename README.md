# rt-voice-wasm

Real-time speech-to-text in the browser and on the server — pluggable STT
engines, streaming pipeline, and a voice agent framework for phone receptionist,
coding companion, and beyond.

## STT engines

Three swappable backends behind a common `SttEngine` trait (`src/engine.rs`):

| Engine | Runtime | Model | Best for |
|--------|---------|-------|----------|
| **Whisper** (ggml) | Statically linked, no deps | `ggml-tiny.en-q5_1.bin` | WASM browser + native |
| **Moonshine** | ONNX Runtime via `libloading` | `sherpa-onnx-moonshine-tiny-en-int8` | Lowest latency, streaming |
| **Parakeet** | ggml via `libloading` | `parakeet_realtime_eou_120m-v1-q8_0.gguf` | End-of-utterance detection |

Engines are loaded at runtime — no recompile to switch. The server selects via
`--provider whisper|moonshine|parakeet`. Cross-engine comparison tests in
`tests/engine_tests.rs`.

## How it works

- Mic audio captured via Web Audio API at 16 kHz mono
- 3-second sliding windows with 1-second step, RMS silence gate
- Inference dispatched to the active STT engine
- Transcript emitted live to the page via WebSocket JSON events
- Optional `--agent-hook` routes transcripts to dirge, an LLM, or any executable

## Quick start — WASM browser demo

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
WEB_ROOT="$PWD/web" jolt -Sdeps '{:paths ["web" "."]}' web/serve.jolt 8000 &
```

Open `http://localhost:8000/index.html` in Chrome or Edge.

## Voice Agent — plug in any brain

The `rt-voice-server` binary is a WebSocket server that accepts raw 16-bit PCM
audio, runs STT inference, and routes the transcript through an **agent** that
decides what to do next. Plug in [dirge](https://dirge-code.github.io/) (or
any script) to drive a phone receptionist, coding agent, or arbitrary
voice-activated interaction.

```bash
# Start the server with dirge as the brain
./scripts/run_receptionist_demo.sh

# Or manually:
cargo build --bin rt-voice-server
./target/debug/rt-voice-server \
    --provider raw \
    --agent-hook './scripts/dirge-agent.sh' \
    --port 8080

# Switch engines:
./target/debug/rt-voice-server --provider moonshine  # ONNX Runtime
./target/debug/rt-voice-server --provider parakeet   # ggml, EOU detection
./target/debug/rt-voice-server --provider whisper    # default, WASM-compatible
```

Open `http://localhost:8000/receptionist.html` — click Connect, speak, and
watch the transcript stream in. The agent routes each utterance through dirge
(using the `phone-receptionist` prompt) and the page speaks responses back
via TTS.

### How the agent hook works

The `--agent-hook` flag runs an arbitrary executable. The server pipes each
transcript line to the process's **stdin** and reads a JSON action from its
**stdout**. Supported actions:

- `{"Respond": "reply text"}` — speak back to the caller
- `{"Transfer": "agent"}` — transfer to a human
- `{"Escalate": "reason"}` — escalate with a reason
- `{"Hangup": null}` — end the call
- `{"Continue": null}` — no action, keep listening

This works with any language or runtime — shell script, Python, Node, a Go
binary, or dirge-code. The `scripts/dirge-agent.sh` script bridges dirge's
JSON output to this protocol, but you can swap it for your own:

```bash
# Use a Python agent
./target/debug/rt-voice-server --agent-hook 'python3 my_router.py'

# Use a compiled agent
./target/debug/rt-voice-server --agent-hook './my-agent --flag'
```

### Agent ideas

- **Coding companion**: `--agent-hook './scripts/dirge-coding-agent.sh'` — speak
  code changes, dirge edits files, runs tests, and reports results by voice
- **Phone receptionist**: `--agent-hook './scripts/dirge-agent.sh'` — routes
  callers, answers FAQs, transfers to humans (default)
- **Home automation**: trigger shell commands from voice commands
- **Dictation**: skip the agent entirely, just stream raw transcripts

### Coding companion

The `dirge-coding-agent.sh` script uses dirge's built-in tools (file read/write,
bash, git, grep, LSP) with the `--accept-all` flag so it can execute commands
without confirmation — essential for hands-free voice interaction.

```bash
# Start the server in coding companion mode
./target/debug/rt-voice-server \
    --provider raw \
    --agent-hook './scripts/dirge-coding-agent.sh'
```

The `voice-coding` prompt (installed to `~/.config/dirge/prompts/voice-coding.md`)
keeps responses terse for TTS: 2-3 spoken sentences, no markdown.

Voice commands dirge can handle:
- "Add a test for the parse function"
- "What does the streaming pipeline do?"
- "Run the tests and tell me how many passed"
- "Rename the handle_connection function to handle_socket"

See [TODOs.md](./TODOs.md) for next steps: flowengine integration, continuous
voice loop, context awareness, multi-modal fallback.

## Architecture

| Layer | File |
|-------|------|
| STT engine trait (swappable backends) | `src/engine.rs` |
| Whisper ggml backend | `src/whisper_host.rs` |
| Moonshine ONNX backend | `src/moonshine.rs` |
| Parakeet ggml backend | `src/parakeet.rs` |
| Audio utilities (μ-law, resampler) | `src/audio.rs` |
| Streaming pipeline (windows, VAD, overlap merge) | `src/stream.rs` |
| Composable call handler | `src/call.rs` |
| Agent system (router, fn, process) | `src/agent.rs` |
| WebSocket transports (Twilio, raw) | `src/transport.rs` |
| WebSocket server binary | `src/bin/server.rs` |
| wasm-bindgen exports | `src/wasm.rs` |
| Module routing | `src/lib.rs` |
| Build (cc + bindgen) | `build.rs` |
| Engine integration tests | `tests/engine_tests.rs` |
| Golden audio regression | `tests/golden_audio.rs` |
| Web harness & voice agent demo | `web/` |
| Convenience scripts | `scripts/` |

## Requirements

- Rust 1.80+
- Emscripten SDK (for whisper.wasm build)
- [Jolt](https://github.com/jolt-lang/jolt) (for dev server and test harness)
- [dirge](https://dirge-code.github.io/) (optional, for the voice agent demo)

## Roadmap

See [TODOs.md](./TODOs.md) — Jolt migration, coding companion, MCP/flowengine integration.
