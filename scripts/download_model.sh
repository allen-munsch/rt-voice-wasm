#!/usr/bin/env bash
set -euo pipefail

MODEL="ggml-tiny.en-q5_1.bin"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL}"
OUTDIR="$(dirname "$0")/../models"

mkdir -p "$OUTDIR"
if [ -f "$OUTDIR/$MODEL" ]; then
    echo "Model already exists: $OUTDIR/$MODEL"
    exit 0
fi

echo "Downloading $MODEL from HuggingFace..."
curl -L --progress-bar -o "$OUTDIR/$MODEL" "$URL"
echo "Done: $OUTDIR/$MODEL"
