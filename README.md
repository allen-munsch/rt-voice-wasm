# rt-voice-wasm

Real-time audio transcription in the browser using
[whisper.cpp](https://github.com/ggerganov/whisper.cpp) compiled to WebAssembly,
with a Rust streaming pipeline and a WebSocket server for phone receptionist /
voice agent workflows.

## How it works

- Mic audio captured via Web Audio API at 16 kHz mono
- 3-second sliding windows with 1-second step, RMS silence gate
- Inference via whisper.cpp (ggml-tiny.en or base.en, quantized)
- Transcript emitted live to the page via WebSocket JSON events

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
python3 web/serve.py
```

Open `http://localhost:8000/index.html` in Chrome or Edge.

## Voice Agent — phone receptionist with dirge

The `rt-voice-server` binary is a WebSocket server that accepts raw 16-bit PCM
audio, runs whisper.cpp inference, and routes the transcript through an
**agent** that decides what to do next. Plug in [dirge](https://dirge.cc) (or
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

- **Coding agent**: `--agent-hook 'dirge --prompt coding-assistant'` — describe
  a bug or feature with your voice, dirge writes the code
- **Customer support**: route transcripts to keyword rules or an LLM for FAQ
  lookup
- **Home automation**: trigger shell commands from voice commands
- **Dictation**: skip the agent entirely, just stream raw transcripts

## Architecture

| Layer | File |
|-------|------|
| Audio utilities (μ-law, resampler) | `src/audio.rs` |
| Streaming pipeline (windows, VAD, overlap merge) | `src/stream.rs` |
| STT engine trait (swappable backends) | `src/engine.rs` |
| whisper.cpp FFI (native) | `src/whisper_host.rs` |
| whisper.cpp FFI (wasm32 stub) | `src/whisper_stub.rs` |
| Composable call handler | `src/call.rs` |
| Agent system (router, fn, process) | `src/agent.rs` |
| WebSocket transports (Twilio, raw) | `src/transport.rs` |
| WebSocket server binary | `src/bin/server.rs` |
| wasm-bindgen exports | `src/wasm.rs` |
| Module routing | `src/lib.rs` |
| Build (cc + bindgen) | `build.rs` |
| Web harness & voice agent demo | `web/` |
| Convenience scripts | `scripts/` |

## Requirements

- Rust 1.80+
- Emscripten SDK (for whisper.wasm build)
- Python 3.8+ (for dev server)
- [dirge](https://dirge.cc) (optional, for the voice agent demo)
