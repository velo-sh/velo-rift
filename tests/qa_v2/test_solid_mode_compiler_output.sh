#!/bin/bash
# Test: Solid Mode - Compiler Output (.o files) in VFS Territory
# RFC-0039: Verifies that build artifacts can be created in VFS directories
# 
# Scenario: Compilation workflow where:
#   1. Source files exist in VFS (manifest HIT)
#   2. Compiler generates .o files in VFS (manifest MISS -> passthrough)
#   3. Close triggers Live Ingest for new files

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SHIM_PATH="$PROJECT_ROOT/target/debug/libvrift_inception_layer.dylib"

# Setup test environment
VFS_DIR="${VRIFT_VFS_PREFIX:-/tmp/test_compiler_output_vfs}"
SRC_DIR="$VFS_DIR/src"
BUILD_DIR="$VFS_DIR/build"

cleanup() {
    rm -rf "$VFS_DIR"
}
trap cleanup EXIT

echo "=== Solid Mode: Compiler Output Test ==="
echo "VFS Directory: $VFS_DIR"
echo "Shim: $SHIM_PATH"
echo ""

# Ensure shim exists
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "❌ FAIL: Shim not found at $SHIM_PATH"
    echo "   Run: cargo build -p vrift-inception-layer"
    exit 1
fi

# Setup
cleanup 2>/dev/null || true
mkdir -p "$SRC_DIR" "$BUILD_DIR"

# Create source file (simulating ingested file)
cat > "$SRC_DIR/hello.c" << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from VFS!\n");
    return 0;
}
EOF

echo "📁 Test setup complete"
echo "   Source: $SRC_DIR/hello.c"
echo "   Build:  $BUILD_DIR/"
echo ""

# Test 1: Compile C file - creates .o in VFS territory
echo "🧪 Test 1: GCC compiles .o file in VFS build directory"
echo "   Command: gcc -c src/hello.c -o build/hello.o"

cd "$VFS_DIR"
export VRIFT_VFS_PREFIX="$VFS_DIR"
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"

if gcc -c src/hello.c -o build/hello.o 2>&1; then
    if [[ -f "$BUILD_DIR/hello.o" ]]; then
        echo "   ✅ PASS: .o file created successfully"
        ls -la "$BUILD_DIR/hello.o"
    else
        echo "   ❌ FAIL: .o file not found"
        exit 1
    fi
else
    echo "   ❌ FAIL: gcc compilation failed"
    exit 1
fi
echo ""

# Test 2: Link .o to executable
echo "🧪 Test 2: GCC links .o to executable"
echo "   Command: gcc build/hello.o -o build/hello"

if gcc build/hello.o -o build/hello 2>&1; then
    if [[ -f "$BUILD_DIR/hello" ]]; then
        echo "   ✅ PASS: Executable created successfully"
        ls -la "$BUILD_DIR/hello"
    else
        echo "   ❌ FAIL: Executable not found"
        exit 1
    fi
else
    echo "   ❌ FAIL: gcc linking failed"
    exit 1
fi
echo ""

# Test 3: Execute the binary
echo "🧪 Test 3: Run compiled binary"
# Note: We need to run without shim to avoid recursion on execution
unset DYLD_INSERT_LIBRARIES
OUTPUT=$("$BUILD_DIR/hello" 2>&1)
if [[ "$OUTPUT" == "Hello from VFS!" ]]; then
    echo "   ✅ PASS: Binary executed correctly"
    echo "   Output: $OUTPUT"
else
    echo "   ❌ FAIL: Unexpected output: $OUTPUT"
    exit 1
fi
echo ""

# Test 4: Incremental build - overwrite .o file
echo "🧪 Test 4: Incremental build (overwrite existing .o)"
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"

# Modify source
cat > "$SRC_DIR/hello.c" << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from VFS v2!\n");
    return 0;
}
EOF

if gcc -c src/hello.c -o build/hello.o 2>&1; then
    echo "   ✅ PASS: .o file overwritten successfully"
else
    echo "   ❌ FAIL: Incremental compile failed"
    exit 1
fi

# Relink and test
if gcc build/hello.o -o build/hello 2>&1; then
    unset DYLD_INSERT_LIBRARIES
    OUTPUT=$("$BUILD_DIR/hello" 2>&1)
    if [[ "$OUTPUT" == "Hello from VFS v2!" ]]; then
        echo "   ✅ PASS: Incremental build worked correctly"
        echo "   Output: $OUTPUT"
    else
        echo "   ❌ FAIL: Unexpected output: $OUTPUT"
        exit 1
    fi
else
    echo "   ❌ FAIL: Incremental link failed"
    exit 1
fi
echo ""

# Summary
echo "================================================================"
echo "✅ ALL TESTS PASSED: Compiler output in VFS works correctly"
echo "================================================================"
echo ""
echo "Verified scenarios:"
echo "  • New .o file creation in VFS territory (manifest MISS)"
echo "  • Linking .o files to executable"
echo "  • Executing compiled binary"
echo "  • Incremental build (overwriting existing .o)"
