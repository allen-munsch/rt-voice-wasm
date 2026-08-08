#!/usr/bin/env bash
# run_scenario_gym.sh — build, start server, run e2e scenario gym, teardown
#
# Usage:
#   ./scripts/run_scenario_gym.sh --dry-run                  # tier-1 Rust replay only
#   ./scripts/run_scenario_gym.sh --tier 2 --agent order     # full e2e with server
#   ./scripts/run_scenario_gym.sh --filter phone --fail-fast # targeted, stop on first fail
#
# Options:
#   --dry-run        Skip server/model; run deterministic tier-1 cargo test only
#   --engine NAME    STT engine: moonshine (default), whisper, parakeet
#   --agent TYPE     Agent: builtin (default) or order
#   --model PATH     Path to model file (default: models/moonshine-tiny)
#   --tier N         Tier gate: 1-3 (default: 1 for dry-run, 2 for full)
#   --filter STR     Only run scenarios whose name contains STR
#   --fail-fast      Stop on first failure
#   --port PORT      Server port (default: 8080)

set -euo pipefail

ENGINE="${ENGINE:-moonshine}"
AGENT="${AGENT:-builtin}"
MODEL="${MODEL:-models/moonshine-tiny}"
PORT="${PORT:-8080}"
TIER=""
FILTER=""
FAIL_FAST=""
DRY_RUN=false

usage() {
    sed -n '2,15p' "$0" | sed 's/^# \?//' | grep -v '^$'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine)   ENGINE="$2"; shift 2 ;;
        --agent)    AGENT="$2"; shift 2 ;;
        --model)    MODEL="$2"; shift 2 ;;
        --port)     PORT="$2"; shift 2 ;;
        --tier)     TIER="$2"; shift 2 ;;
        --filter)   FILTER="$2"; shift 2 ;;
        --fail-fast) FAIL_FAST="--fail-fast"; shift ;;
        --dry-run)  DRY_RUN=true; shift ;;
        --help|-h)  usage ;;
        *) echo "Unknown flag: $1"; usage ;;
    esac
done

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

JOLT_BIN="${JOLT:-jolt}"
HARNESS="${HARNESS:-scripts/tts_harness.jolt}"

echo "=== Scenario Gym ==="
echo "Engine: $ENGINE | Agent: $AGENT | Tier: ${TIER:-default} | Port: $PORT"

# -- Dry-run: deterministic tier-1 replay only ----------------------------

if $DRY_RUN; then
    echo ""
    echo "--- Dry-run: deterministic tier-1 replay ---"
    cargo test --test routing_market 2>&1
    echo ""
    echo "=== Dry-run complete ==="
    exit 0
fi

# -- Full e2e: build, start server, run harness ---------------------------

echo ""

# Kill anything already on our port
fuser -k "${PORT}/tcp" 2>/dev/null || true
sleep 0.5

# Build
echo "--- Building ---"
cargo build --bin rt-voice-server --bin ws-send 2>&1 | tail -3

# Start server
echo "--- Starting server ---"
./target/debug/rt-voice-server \
    --provider raw --port "$PORT" \
    --engine "$ENGINE" --model "$MODEL" --agent "$AGENT" &
SERVER_PID=$!
sleep 2

# Verify server is up
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "ERROR: server failed to start"
    exit 1
fi

trap 'echo ""; echo "--- Teardown ---"; kill $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null; echo "Done."' EXIT

# Build harness args
HARNESS_ARGS=()
[[ -n "$TIER" ]] && HARNESS_ARGS+=(--tier "$TIER")
[[ -n "$FILTER" ]] && HARNESS_ARGS+=(--filter "$FILTER")
[[ -n "$FAIL_FAST" ]] && HARNESS_ARGS+=(--fail-fast)

# Run scenario gym
echo "--- Running scenarios ---"
export SCENARIOS="$PROJECT_DIR/scenarios"
export WS_SERVER="ws://localhost:${PORT}"
export WS_SEND_BIN="$PROJECT_DIR/target/debug/ws-send"

$JOLT_BIN "$HARNESS" "${HARNESS_ARGS[@]}"

echo ""
echo "=== Scenario gym complete ==="
