#!/usr/bin/env bash
# Phone Receptionist Demo — starts server + web UI.
# Requires: cargo, jolt, whisper model at models/ggml-tiny.en-q5_1.bin
#   jolt:  curl -sL https://raw.githubusercontent.com/jolt-lang/jolt/main/install | bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Kill any old instances
pkill -f rt-voice-server 2>/dev/null || true
fuser -k 8080/tcp 2>/dev/null || true
fuser -k 8000/tcp 2>/dev/null || true
sleep 1

cleanup() {
    echo ""
    echo "Shutting down..."
    kill $SERVER_PID 2>/dev/null || true
    kill $HTTP_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
    wait $HTTP_PID 2>/dev/null || true
}
trap cleanup EXIT INT TERM

cd "$PROJECT_DIR"

echo "=== Phone Receptionist Demo ==="
echo ""
echo "  1. rt-voice-server  → ws://localhost:8080 (Whisper + builtin agent)"
echo "  2. Static files      → http://localhost:8000"
echo ""
echo "  Open http://localhost:8000/receptionist.html"
echo "  Click 'Connect & Start Mic', speak for 3+ seconds."
echo ""

# Start server (uses builtin keyword-router agent)
LD_LIBRARY_PATH="$PROJECT_DIR/build/parakeet:$PROJECT_DIR/build/moonshine:$LD_LIBRARY_PATH" \
./target/debug/rt-voice-server \
    --provider raw \
    --port 8080 \
    &
SERVER_PID=$!

# Start static file server (jolt native ServerSocket)
WEB_ROOT="$PROJECT_DIR/web" jolt -Sdeps '{:paths ["web" "."]}' "$PROJECT_DIR/web/serve.jolt" 8000 &
HTTP_PID=$!

echo "Server PID: $SERVER_PID  HTTP PID: $HTTP_PID"
echo "Waiting for server to be ready..."
sleep 3

# Open browser (Linux)
if command -v xdg-open &>/dev/null; then
    xdg-open http://localhost:8000/receptionist.html 2>/dev/null || true
elif command -v open &>/dev/null; then
    open http://localhost:8000/receptionist.html 2>/dev/null || true
fi

echo "Press Ctrl+C to stop."
wait
