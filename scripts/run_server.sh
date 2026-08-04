#!/usr/bin/env bash
# Launch the Twilio Media Streams → Whisper transcription server
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
MODEL="${MODEL:-$PROJECT_DIR/models/ggml-tiny.en-q5_1.bin}"
PORT="${PORT:-8080}"

cd "$PROJECT_DIR"

if [ ! -f "$MODEL" ]; then
    echo "Model not found at $MODEL"
    echo "Run: ./scripts/download_model.sh"
    exit 1
fi

echo "Building server..."
cargo build --release --bin rt-voice-server 2>&1 | tail -3

exec "$PROJECT_DIR/target/release/rt-voice-server" --model "$MODEL" --port "$PORT"
