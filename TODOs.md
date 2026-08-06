# TODOs

## Jolt migration (Python to Jolt) — SHIPPED

**Shipped (2025-08-05):**

- [x] `web/serve.jolt` — native HTTP server using `java.net.ServerSocket` via `jolt/socket.clj` (POSIX FFI), no socat needed. COOP/COEP headers, correct MIME types for .wasm/.js/.html. Invoked by `scripts/run_receptionist_demo.sh`: `jolt -Sdeps '{:paths ["web" "."]}' web/serve.jolt 8000 &`
- [x] `scripts/tts_harness.jolt` — closed-loop voice scenario harness. Orchestrates: python3 edge-tts one-liner → ffmpeg (16kHz mono WAV) → repo's `ws-send` binary → jq event matching. Five scenarios from `tests/scenarios.json`.
- [x] `java.net.Socket / ServerSocket / InetSocketAddress` — shipped as `jolt/socket.clj` using POSIX FFI (`jolt.ffi`). No Jolt rebuild needed; `__register-class-ctor!` and `__register-class-methods!` work at runtime.
- [x] `jolt/socket.clj` vendored at both `jolt/socket.clj` and `web/jolt/socket.clj` so `web/serve.jolt` finds it on `{:paths ["web" "."]}`
- [x] Python 3.8+ requirement removed from README

**Known remaining gaps (candidates for upstream Jolt issues):**

- [] `ServerSocket.getLocalPort` on port 0 — `getsockname` not bound; OS-assigned port doesn't report
- [] Concurrent accept — single-threaded (non-blocking accept would need `fcntl O_NONBLOCK` + poll)

**Deleted:** `web/serve.py`, `scripts/tts_harness.py`, `scripts/__pycache__/`. `scripts/convert_moonshine_model.py` kept.

---

## Coding companion (voice-driven programming)

Move beyond phone receptionist into a coding companion that runs configured
MCP tools and [flowengine](https://github.com/allen-munsch/flowengine) so the
user can program and interact by voice alone.

The `--agent-hook` protocol already supports this: any executable that reads
transcripts from stdin and emits JSON actions on stdout can be the brain.
Today that's `scripts/dirge-agent.sh` → dirge with `phone-receptionist` prompt.
Tomorrow it can be dirge with a `coding-assistant` prompt, backed by MCP tools
and flowengine for computer use.

- [x] **MCP tool bridge for dirge** — shipped. `scripts/dirge-coding-agent.sh` + `~/.config/dirge/prompts/voice-coding.md`. Dirge reads voice transcripts via `--prompt voice-coding --accept-all`, uses its built-in tools (read/edit/write/bash/grep/lsp), returns TTS-friendly voice responses via the ProcessAgent protocol. Test: `cargo test process_agent_with_dirge_coding_agent` (1 pass). Usage: `rt-voice-server --agent-hook './scripts/dirge-coding-agent.sh'`
- [] **flowengine integration** — dirge uses flowengine for desktop automation (open editor, navigate, type, click) driven by voice commands
- [] **Continuous voice loop** — after each response, keep the mic open so the user can iterate without touching the keyboard: "rename this function" → dirge edits → "now add a test" → dirge writes test → "run it" → dirge runs and reports
- [] **Context awareness** — dirge has access to the current file, project structure, and git status so voice commands are relative ("add a parameter to this function") rather than absolute
- [] **Multi-modal fallback** — when voice transcription confidence is low or the command is ambiguous, dirge asks for clarification (voice or typed)
