#!/bin/bash
# QA Test: Cargo Build with Shim - Stability Test
# This test verifies that cargo can function normally when shim is loaded
# EXPECTED: cargo build should complete successfully
# KNOWN ISSUE: cargo crashes with "Failed building the Runtime"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
TEST_DIR=$(mktemp -d)

echo "=== QA Test: Cargo + Shim Stability ==="

# Check if shim exists
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "❌ SKIP: Shim not built at $SHIM_PATH"
    rm -rf "$TEST_DIR"
    exit 0
fi

# Create a minimal Rust project
cd "$TEST_DIR"
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    cargo init --name shimtest 2>/dev/null
)

if [[ ! -f "$TEST_DIR/Cargo.toml" ]]; then
    echo "❌ SKIP: Failed to create test project"
    rm -rf "$TEST_DIR"
    exit 0
fi

echo "Test project created at: $TEST_DIR"

# Test: Can cargo build with shim loaded?
echo "Testing: cargo build with DYLD_INSERT_LIBRARIES..."
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1

OUTPUT=$(cargo build 2>&1)
EXIT_CODE=$?

unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE

rm -rf "$TEST_DIR"

if [[ $EXIT_CODE -eq 0 ]]; then
    echo "✅ PASS: cargo build completed successfully with shim"
    exit 0
else
    echo "❌ FAIL: cargo build failed with shim loaded"
    echo "Exit code: $EXIT_CODE"
    echo "Output (last 10 lines):"
    echo "$OUTPUT" | tail -10
    echo ""
    echo "KNOWN ISSUES (should be fixed):"
    echo "- Variadic ABI: open/openat/fcntl cannot be safely interposed"
    exit 1
fi
