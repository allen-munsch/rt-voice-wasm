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
`--engine whisper|moonshine|parakeet`; `--provider` selects the audio transport
(twilio|vapi|raw). Cross-engine comparison tests in
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
./target/debug/rt-voice-server --engine moonshine  # ONNX Runtime (default)
./target/debug/rt-voice-server --engine parakeet   # ggml, EOU detection
./target/debug/rt-voice-server --engine whisper    # WASM-compatible
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

### Built-in agents

Two built-in agents are available without an external process, selected via
`--agent builtin` (default) or `--agent order`:

- **IntentRouter** (`builtin`) — first-match-wins keyword/phrase routing with
  configurable rules. Single-word triggers match whole words only (stripping
  punctuation: `"agent!"` matches `"agent"`), while multi-word phrases use
  substring matching (`"not working"` in `"it is not working"`). Falls back
  to a prompt if nothing matches.
- **OrderFlowAgent** (`order`) — deterministic state machine for voice-driven
  orders: slot fill (item → size → confirm) → structured JSON payload. Handles
  corrections (`"actually make it three"`), add/remove items, cancel, and
  unknown-item clarification. Quantities parsed from words (`"two lattes"`)
  with word-boundary safety (`"someone"` doesn't match `"one"`).

```bash
# IntentRouter (default — phone receptionist routing)
./target/debug/rt-voice-server --provider raw --agent builtin

# OrderFlowAgent (voice-driven café ordering)
./target/debug/rt-voice-server --provider raw --agent order
```

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

## Scenario Gym

A catalog of 50+ voice-to-action scenarios in `scenarios/` — spoken phrases
paired with expected agent events. Three tiers of testing:

| Tier | Mode | Runtime |
|------|------|---------|
| 1 | Deterministic Rust replay | `cargo test --test routing_market` |
| 2 | E2E: TTS → server → STT → assert | `jolt scripts/tts_harness.jolt --tier 2` |
| 3 | Dirge/LLM + tool effects | Tier 2 + `effect` assertions |

**Domain files:** `phone-routing.json`, `order-taking.json`, `coding-companion.json`,
`cli-programmer.json`, `tool-calling-flowengine.json`, `tool-calling-mcp.json`,
`a2a.json`, `ux-quality.json`.

```bash
# One-shot orchestrator
./scripts/run_scenario_gym.sh --dry-run        # tier-1 Rust replay only
./scripts/run_scenario_gym.sh --tier 2 --agent order  # full e2e

# Targeted run
jolt scripts/tts_harness.jolt --tier 1 --filter phone
```

The `ws-send` binary supports `--ts` for per-event latency instrumentation:

```bash
./target/debug/ws-send ws://localhost:8080 test.wav --ts
# Output: T 123 {"event":"agent_action","text":"transfer to agent"}
#          ^^^ ms since connect
```

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
| Agent system (IntentRouter, OrderFlow, Process) | `src/agent.rs` |
| WebSocket transports (Twilio, raw) | `src/transport.rs` |
| WebSocket server binary | `src/bin/server.rs` |
| ws-send latency probe binary | `src/bin/ws_send.rs` |
| wasm-bindgen exports | `src/wasm.rs` |
| Module routing | `src/lib.rs` |
| Build (cc + bindgen) | `build.rs` |
| Engine integration tests | `tests/engine_tests.rs` |
| Golden audio regression | `tests/golden_audio.rs` |
| CallHandler integration tests | `tests/call_handler.rs` |
| Deterministic scenario replay | `tests/routing_market.rs` |
| Scenario definitions (50+ turns) | `scenarios/` |
| Web harness & voice agent demo | `web/` |
| Convenience scripts | `scripts/` |

## Requirements

- Rust 1.80+
- Emscripten SDK (for whisper.wasm build)
- [Jolt](https://github.com/jolt-lang/jolt) (for dev server and test harness)
- [dirge](https://dirge-code.github.io/) (optional, for the voice agent demo)
- Python 3 + edge-tts (for tier-2+ scenario gym)

## Roadmap

- Scenario Gym tier-3: dirge/LLM agents + effect assertions (file_exists, http)
- Flowengine DAG orchestration from voice commands
- Multi-turn context awareness in streaming pipeline
- See [TODOs.md](./TODOs.md) for full backlog
