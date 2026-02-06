#!/bin/bash
# Test: Shim Initialization Hang Detection
# Purpose: Verify inception can be injected into binaries without deadlock
# Priority: P0 - This is a blocker for all shell command interception
#
# This test MUST pass for shell-based tests to work (chmod, rm, mv, etc.)
# If this test hangs, the inception has fundamental initialization issues.

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== P0 Test: Shim Initialization (No Deadlock) ==="

cleanup() {
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Create test binaries from source (arm64e compatible)
# Copying /bin/echo doesn't work due to arm64e pointer signing
cat > "$TEST_DIR/echo.c" << 'EOF'
#include <stdio.h>
int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        printf("%s%s", argv[i], i < argc - 1 ? " " : "");
    }
    printf("\n");
    return 0;
}
EOF
cc -O2 -o "$TEST_DIR/echo" "$TEST_DIR/echo.c"
rm -f "$TEST_DIR/echo.c"

cat > "$TEST_DIR/chmod.c" << 'EOF'
#include <sys/stat.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc < 3) return 1;
    mode_t mode = strtol(argv[1], NULL, 8);
    return chmod(argv[2], mode);
}
EOF
cc -O2 -o "$TEST_DIR/chmod" "$TEST_DIR/chmod.c"
rm -f "$TEST_DIR/chmod.c"

# Sign for DYLD_INSERT_LIBRARIES
codesign -s - -f "$TEST_DIR/echo" 2>/dev/null || true
codesign -s - -f "$TEST_DIR/chmod" 2>/dev/null || true

# Prefer release inception, fallback to debug
RELEASE_INCEPTION="${PROJECT_ROOT}/target/release/libvrift_inception.dylib"
DEBUG_INCEPTION="${PROJECT_ROOT}/target/debug/libvrift_inception.dylib"

if [[ -f "$RELEASE_INCEPTION" ]]; then
    INCEPTION_PATH="$RELEASE_INCEPTION"
    echo "Using RELEASE inception: $INCEPTION_PATH"
elif [[ -f "$DEBUG_INCEPTION" ]]; then
    INCEPTION_PATH="$DEBUG_INCEPTION"
    echo "Using DEBUG inception: $INCEPTION_PATH"
else
    echo "❌ FAIL: Shim library not found in target/release or target/debug."
    exit 1
fi

echo "[1] Testing echo with inception (simplest case)..."
# echo should complete immediately - no file operations
# Use background process with timeout instead of perl alarm
RESULT=$( (DYLD_INSERT_LIBRARIES="$INCEPTION_PATH" DYLD_FORCE_FLAT_NAMESPACE=1 \
    "$TEST_DIR/echo" "hello" &
    PID=$!
    sleep 5 && kill -9 $PID 2>/dev/null &
    wait $PID 2>/dev/null) )
EXIT_CODE=$?

if [[ "$RESULT" == "hello" ]]; then
    echo "✅ PASS: echo with inception works"
else
    echo "❌ FAIL: echo with inception failed or hung (Exit Code: ${EXIT_CODE:-0})"
    echo "   Output: $RESULT"
    echo "   Diagnosis: If output is empty and it took 5s, it is a dyld-level deadlock."
    exit 1
fi

echo ""
echo "[2] Testing chmod with inception (mutation syscall)..."
echo "test" > "$TEST_DIR/testfile.txt"
# chmod should complete - if it hangs, inception has initialization deadlock
(DYLD_INSERT_LIBRARIES="$INCEPTION_PATH" DYLD_FORCE_FLAT_NAMESPACE=1 \
    "$TEST_DIR/chmod" 644 "$TEST_DIR/testfile.txt" &
    PID=$!
    (sleep 3 && kill -9 $PID 2>/dev/null) &
    TIMEOUT_PID=$!
    wait $PID 2>/dev/null
    EXIT_CODE=$?
    kill $TIMEOUT_PID 2>/dev/null
    exit $EXIT_CODE
) || {
    EXIT_CODE=$?
    if [[ $EXIT_CODE -eq 137 ]]; then
        echo "❌ FAIL: chmod with inception HUNG (killed by timeout)"
        exit 1
    else
        echo "chmod exited with code $EXIT_CODE (may be expected)"
    fi
}

echo "✅ PASS: chmod with inception completed (no hang)"

echo ""
echo "[3] Testing chmod with VFS_PREFIX set..."
mkdir -p "$TEST_DIR/workspace/.vrift"
echo "protected" > "$TEST_DIR/workspace/protected.txt"

(DYLD_INSERT_LIBRARIES="$INCEPTION_PATH" DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_VFS_PREFIX="$TEST_DIR/workspace" \
    "$TEST_DIR/chmod" 644 "$TEST_DIR/workspace/protected.txt" &
    PID=$!
    (sleep 3 && kill -9 $PID 2>/dev/null) &
    TIMEOUT_PID=$!
    wait $PID 2>/dev/null
    EXIT_CODE=$?
    kill $TIMEOUT_PID 2>/dev/null
    exit $EXIT_CODE
) || {
    EXIT_CODE=$?
    if [[ $EXIT_CODE -eq 137 ]]; then
        echo "❌ FAIL: chmod with VFS_PREFIX HUNG"
        exit 1
    fi
}

echo "✅ PASS: chmod with VFS_PREFIX completed"

echo ""
echo "=== All inception initialization tests passed ==="
exit 0

