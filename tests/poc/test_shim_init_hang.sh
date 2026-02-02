#!/bin/bash
# Test: Shim Initialization Hang Detection
# Purpose: Verify shim can be injected into binaries without deadlock
# Priority: P0 - This is a blocker for all shell command interception
#
# This test MUST pass for shell-based tests to work (chmod, rm, mv, etc.)
# If this test hangs, the shim has fundamental initialization issues.

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== P0 Test: Shim Initialization (No Deadlock) ==="

cleanup() {
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Copy a simple binary to bypass SIP
cp /bin/echo "$TEST_DIR/echo"
cp /bin/chmod "$TEST_DIR/chmod"
codesign -s - -f "$TEST_DIR/echo" 2>/dev/null || true
codesign -s - -f "$TEST_DIR/chmod" 2>/dev/null || true

# Prefer release shim, fallback to debug
RELEASE_SHIM="${PROJECT_ROOT}/target/release/libvrift_shim.dylib"
DEBUG_SHIM="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"

if [[ -f "$RELEASE_SHIM" ]]; then
    SHIM_PATH="$RELEASE_SHIM"
    echo "Using RELEASE shim: $SHIM_PATH"
elif [[ -f "$DEBUG_SHIM" ]]; then
    SHIM_PATH="$DEBUG_SHIM"
    echo "Using DEBUG shim: $SHIM_PATH"
else
    echo "❌ FAIL: Shim library not found in target/release or target/debug."
    exit 1
fi

echo "[1] Testing echo with shim (simplest case)..."
# echo should complete immediately - no file operations
# We use perl for a hard timeout because macOS lacks GNU timeout
RESULT=$(DYLD_INSERT_LIBRARIES="$SHIM_PATH" DYLD_FORCE_FLAT_NAMESPACE=1 \
    perl -e 'alarm 5; exec @ARGV' "$TEST_DIR/echo" "hello" 2>&1) || EXIT_CODE=$?

if [[ "$RESULT" == "hello" ]]; then
    echo "✅ PASS: echo with shim works"
else
    echo "❌ FAIL: echo with shim failed or hung (Exit Code: ${EXIT_CODE:-0})"
    echo "   Output: $RESULT"
    echo "   Diagnosis: If output is empty and it took 5s, it is a dyld-level deadlock."
    exit 1
fi

echo ""
echo "[2] Testing chmod with shim (mutation syscall)..."
echo "test" > "$TEST_DIR/testfile.txt"
# chmod should complete - if it hangs, shim has initialization deadlock
DYLD_INSERT_LIBRARIES="$SHIM_PATH" DYLD_FORCE_FLAT_NAMESPACE=1 \
    perl -e 'alarm 3; exec @ARGV' "$TEST_DIR/chmod" 644 "$TEST_DIR/testfile.txt" 2>&1 || {
    EXIT_CODE=$?
    if [[ $EXIT_CODE -eq 142 ]]; then
        echo "❌ FAIL: chmod with shim HUNG (SIGALRM timeout)"
        echo ""
        echo "=== DIAGNOSIS ==="
        echo "The shim initialization causes a deadlock when injected into chmod."
        echo "This is a P0 bug in the shim's dyld interposition."
        echo ""
        echo "Likely causes:"
        echo "  1. Dangerous syscall interposed (dlopen/dlsym/malloc during init)"
        echo "  2. ShimState::get() triggers allocation during dyld bootstrap"
        echo "  3. Recursion in constructor or interpose table setup"
        exit 1
    else
        echo "chmod exited with code $EXIT_CODE (may be expected)"
    fi
}

echo "✅ PASS: chmod with shim completed (no hang)"

echo ""
echo "[3] Testing chmod with VFS_PREFIX set..."
mkdir -p "$TEST_DIR/workspace/.vrift"
echo "protected" > "$TEST_DIR/workspace/protected.txt"

DYLD_INSERT_LIBRARIES="$SHIM_PATH" DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_VFS_PREFIX="$TEST_DIR/workspace" \
    perl -e 'alarm 3; exec @ARGV' "$TEST_DIR/chmod" 644 "$TEST_DIR/workspace/protected.txt" 2>&1 || {
    EXIT_CODE=$?
    if [[ $EXIT_CODE -eq 142 ]]; then
        echo "❌ FAIL: chmod with VFS_PREFIX HUNG"
        exit 1
    fi
}

echo "✅ PASS: chmod with VFS_PREFIX completed"

echo ""
echo "=== All shim initialization tests passed ==="
exit 0
