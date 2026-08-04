#!/usr/bin/env bash
set -euo pipefail

FIXTURE="jfk.wav"
URL="https://raw.githubusercontent.com/ggerganov/whisper.cpp/master/samples/${FIXTURE}"
OUTDIR="$(dirname "$0")/../tests/fixtures"

mkdir -p "$OUTDIR"
if [ -f "$OUTDIR/$FIXTURE" ]; then
    echo "Fixture already exists: $OUTDIR/$FIXTURE"
    exit 0
fi

echo "Downloading $FIXTURE..."
curl -L --progress-bar -o "$OUTDIR/$FIXTURE" "$URL"
echo "Done: $OUTDIR/$FIXTURE"
