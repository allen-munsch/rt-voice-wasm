#!/usr/bin/env bash
# Build parakeet.cpp as a shared library with only the C API symbols exported.
# The version script hides all ggml internals so they don't conflict with
# whisper.cpp's statically-linked ggml.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
PARAKEET_DIR="$PROJECT_DIR/third_party/parakeet.cpp"
BUILD_DIR="$PARAKEET_DIR/build"
INSTALL_DIR="$PROJECT_DIR/build/parakeet"

# Version script: only export parakeet_capi_* symbols
EXPORTS_FILE="$BUILD_DIR/exports.map"
mkdir -p "$BUILD_DIR"
cat > "$EXPORTS_FILE" << 'EOF'
{
    global: parakeet_capi_*;
    local: *;
};
EOF

cmake -B "$BUILD_DIR" \
    -DPARAKEET_BUILD_CLI=OFF \
    -DPARAKEET_BUILD_SERVER=OFF \
    -DPARAKEET_BUILD_TESTS=OFF \
    -DPARAKEET_SHARED=ON \
    -DGGML_NATIVE=OFF \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--version-script=$EXPORTS_FILE" \
    "$PARAKEET_DIR"

cmake --build "$BUILD_DIR" -j$(nproc)

# Install .so files
mkdir -p "$INSTALL_DIR"
cp -a "$BUILD_DIR/libparakeet.so" "$INSTALL_DIR/"
for lib in libggml.so libggml-base.so libggml-cpu.so; do
    # Copy the versioned symlink chain
    cp -a "$BUILD_DIR/third_party/ggml/src/$lib"* "$INSTALL_DIR/" 2>/dev/null || true
done

echo "Parakeet built. Libraries:"
ls -lh "$INSTALL_DIR/"
echo ""
echo "Add to LD_LIBRARY_PATH or link with:"
echo "  export LD_LIBRARY_PATH=$INSTALL_DIR:\$LD_LIBRARY_PATH"
