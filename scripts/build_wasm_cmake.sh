#!/usr/bin/env bash
# Build whisper.wasm using the official emcmake build
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
WHISPER_DIR="$PROJECT_DIR/third_party/whisper.cpp"
BUILD_DIR="$WHISPER_DIR/build-wasm"
OUT_DIR="$PROJECT_DIR/web/whisper"

source /tmp/emsdk/emsdk_env.sh 2>/dev/null

export PATH="$HOME/.local/bin:$PATH"

mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

echo "Configuring whisper.cpp with emcmake..."
emcmake cmake .. \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DWHISPER_BUILD_EXAMPLES=ON \
  -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF \
  2>&1 | tail -5

echo "Building..."
emmake make -j$(nproc) 2>&1 | tail -10

# Check for whisper.wasm output
echo "Looking for built artifacts..."
find "$BUILD_DIR" -name "*.wasm" -o -name "*.js" 2>/dev/null | grep -v node_modules | head -10

# Copy to web/whisper
echo "Copying to $OUT_DIR..."
mkdir -p "$OUT_DIR"
# Find the example output
if [ -f "$BUILD_DIR/bin/whisper.wasm.js" ]; then
    cp "$BUILD_DIR/bin/"*.js "$BUILD_DIR/bin/"*.wasm "$BUILD_DIR/bin/"*.data "$OUT_DIR/" 2>/dev/null || true
elif [ -f "$BUILD_DIR/examples/whisper.wasm/whisper.wasm.js" ]; then
    cp "$BUILD_DIR/examples/whisper.wasm/"*.js "$BUILD_DIR/examples/whisper.wasm/"*.wasm "$BUILD_DIR/examples/whisper.wasm/"*.data "$OUT_DIR/" 2>/dev/null || true
fi

echo "Done!"
ls -lh "$OUT_DIR/"*.wasm "$OUT_DIR/"*.js 2>/dev/null || echo "No output files found"
