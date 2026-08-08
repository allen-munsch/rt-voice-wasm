# Scenario Gym Marketplace

Voice-to-action scenario catalog for the rt-voice-wasm framework. Each domain file
contains a `scenarios` array of turns — spoken phrases paired with expected events.

## Quick start

```bash
# Terminal 1: start server (model required)
cargo run --bin rt-voice-server -- --provider raw --agent order

# Terminal 2: run all tier-2 scenarios via TTS → STT → assert
SCENARIOS=scenarios jolt scripts/tts_harness.jolt --tier 2

# Or run just one domain
SCENARIOS=scenarios/phone-routing.json jolt scripts/tts_harness.jolt --tier 1

# Rust-only deterministic replay (no model, no network)
cargo test --test routing_market
```

## Tier system

| Tier | Name | What runs | Requires |
|------|------|-----------|----------|
| 1 | Deterministic replay | Rust `CallHandler` + `CannedEngine`, no model | `cargo test` only |
| 2 | E2E with model | edge-tts → ws-send → live server → latency budgets | Live model + server |
| 3 | Dirge/LLM + tools | Tier 2 + external process agent + effect assertions | Dirge, flowengine, MCP |

## Schema v2

Each scenario file is JSON with a `scenarios` array. Each scenario:

```json
{
  "name": "route-to-agent",
  "description": "Customer asks for a human agent",
  "tier": 1,
  "tags": ["routing", "transfer"],
  "voice": "en-US-AriaNeural",
  "rate": "+0%",
  "pitch": "+0Hz",
  "pause_ms": 500,
  "turns": [
    {
      "speak": "I want to speak to a human agent please",
      "expect": {
        "event": "agent_action",
        "contains": "transfer",
        "within_ms": 3000,
        "not_contains": "error"
      }
    }
  ],
  "effect": {
    "file_exists": null,
    "command": null,
    "http": null
  }
}
```

### Fields

- `tier` — 1/2/3 (see table above)
- `tags` — freeform labels for filtering
- `voice` / `rate` / `pitch` — edge-tts voice parameters (tier 2+)
- `pause_ms` — silence gap between turns, simulating barge-in / turn-taking
- `expect.event` — event kind to match (`agent_action`, `transcript`, `state`, `error`, `full_transcript`)
- `expect.contains` — substring the event text must contain
- `expect.within_ms` — max latency from audio send to event arrival (tier 2+)
- `expect.not_contains` — forbidden substring in any event (glitch gate)
- `effect` — downstream side-effect assertions (tier 3):
  - `file_exists` — path that must exist after the scenario
  - `command` + `contains` — shell command whose output must contain text
  - `http` — URL + expected status code

### Adding a scenario

1. Pick the right domain file (or create a new one)
2. Write `turns` with `speak`/`expect` pairs
3. Set `tier`: 1 for deterministic, 2 for model-only, 3 for dirge/tools
4. Run `jq empty scenarios/your-file.json` to validate syntax
5. For tier 1, add matching `CannedEngine` texts in `tests/routing_market.rs`

## Domain index

| File | Domain | Scenario count | Highest tier |
|------|--------|---------------|-------------|
| `phone-routing.json` | Call center receptionist | 14 | 2 |
| `order-taking.json` | Voice-driven order flow | 10 | 3 |
| `coding-companion.json` | Voice coding with dirge | 8 | 3 |
| `cli-programmer.json` | Voice-to-shell commands | 6 | 3 |
| `tool-calling-flowengine.json` | Flowengine DAG orchestration | 6 | 3 |
| `tool-calling-mcp.json` | MCP tool invocation | 6 | 3 |
| `a2a.json` | Agent-to-agent handoff | 6 | 3 |
| `ux-quality.json` | Latency and robustness gates | 8 | 2 |
