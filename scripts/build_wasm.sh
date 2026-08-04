#!/usr/bin/env bash
# Build whisper.wasm with emcc (no cmake required)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
WHISPER_DIR="$PROJECT_DIR/third_party/whisper.cpp"
OUT_DIR="$PROJECT_DIR/web/whisper"

source /tmp/emsdk/emsdk_env.sh 2>/dev/null

mkdir -p "$OUT_DIR"

INC_DIRS=(
  "-I$WHISPER_DIR/include"
  "-I$WHISPER_DIR/ggml/include"
  "-I$WHISPER_DIR/src"
  "-I$WHISPER_DIR/ggml/src"
  "-I$WHISPER_DIR/ggml/src/ggml-cpu"
  "-I$WHISPER_DIR/ggml/src/ggml-cpu/arch/wasm"
)

DEFINES=(
  -DGGML_USE_CPU
  '-DWHISPER_VERSION="1.9.1"'
  -DGGML_SCHED_MAX_COPIES=4
  '-DGGML_VERSION="1.9.1"'
  '-DGGML_COMMIT="unknown"'
)

# Source files
C_FILES=(
  "$WHISPER_DIR/ggml/src/ggml.c"
  "$WHISPER_DIR/ggml/src/ggml-alloc.c"
  "$WHISPER_DIR/ggml/src/ggml-cpu/quants.c"
  "$WHISPER_DIR/ggml/src/ggml-cpu/arch/wasm/quants.c"
)

CPP_FILES=(
  "$WHISPER_DIR/src/whisper.cpp"
  "$WHISPER_DIR/ggml/src/ggml.cpp"
  "$WHISPER_DIR/ggml/src/ggml-backend.cpp"
  "$WHISPER_DIR/ggml/src/ggml-backend-meta.cpp"
  "$WHISPER_DIR/ggml/src/ggml-backend-reg.cpp"
  "$WHISPER_DIR/ggml/src/ggml-opt.cpp"
  "$WHISPER_DIR/ggml/src/ggml-threading.cpp"
  "$WHISPER_DIR/ggml/src/gguf.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/ggml-cpu.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/repack.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/traits.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/vec.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/ops.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/binary-ops.cpp"
  "$WHISPER_DIR/ggml/src/ggml-cpu/unary-ops.cpp"
)

EMCC_FLAGS=(
  -O3
  -s WASM=1
  -s ALLOW_MEMORY_GROWTH=1
  -s INITIAL_MEMORY=256MB
  -s MAXIMUM_MEMORY=2000MB
  -s MODULARIZE=1
  -s EXPORT_NAME='"WhisperModule"'
  -s EXPORTED_FUNCTIONS='["_wasm_init_from_file","_wasm_free","_wasm_full_default_params","_wasm_full","_wasm_full_n_segments","_wasm_full_get_segment_text","_wasm_full_get_segment_t0","_wasm_full_get_segment_t1","_malloc","_free"]'
  -s EXPORTED_RUNTIME_METHODS='["ccall","cwrap","HEAPU8","HEAPF32","HEAP32","FS"]'
  -s FORCE_FILESYSTEM=1
  -s SINGLE_FILE=0
  --no-entry
)

echo "Building whisper.wasm..."

emcc "${EMCC_FLAGS[@]}" "${INC_DIRS[@]}" "${DEFINES[@]}" \
  "${C_FILES[@]}" "${CPP_FILES[@]}" \
  "$PROJECT_DIR/web/whisper/bridge.c" \
  -o "$OUT_DIR/whisper.js"

echo "Done: $OUT_DIR/whisper.js"
ls -lh "$OUT_DIR/"*.js "$OUT_DIR/"*.wasm 2>/dev/null || true
