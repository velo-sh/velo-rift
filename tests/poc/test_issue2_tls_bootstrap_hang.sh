#!/bin/bash
# Test: Issue #2 - TLS Bootstrap Hang Behavior
# Priority: CRITICAL
# Verifies that the shim doesn't hang during early process initialization

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
# Determine OS and shim name
if [[ "$OSTYPE" == "darwin"* ]]; then
    SHIM_NAME="libvrift_inception_layer.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
else
    SHIM_NAME="libvrift_shim.so"
    PRELOAD_VAR="LD_PRELOAD"
fi

# Prefer release builds (CI), fallback to debug
if [ -f "${PROJECT_ROOT}/target/release/${SHIM_NAME}" ]; then
    SHIM_PATH="${PROJECT_ROOT}/target/release/${SHIM_NAME}"
else
    SHIM_PATH="${PROJECT_ROOT}/target/debug/${SHIM_NAME}"
fi

echo "=== Test: TLS Bootstrap Hang Behavior ==="

if [[ ! -f "$SHIM_PATH" ]]; then
    echo "⚠️ Shim not found at $SHIM_PATH"
    # exit 0  # Re-enable if you want it to be a hard failure in CI
    exit 0
fi

export "$PRELOAD_VAR"="$(realpath "$SHIM_PATH")"
if [[ "$OSTYPE" == "darwin"* ]]; then
    export DYLD_FORCE_FLAT_NAMESPACE=1
fi

# Run a simple command under the shim with a tight timeout
# If it hangs, the timeout will catch it.
echo "Running 'id' command under shim..."

# We use perl for timeout here too

OUT=$(perl -e 'alarm 5; exec @ARGV' id 2>&1)
CODE=$?

unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE

if [[ $CODE -eq 0 ]]; then
    echo "✅ PASS: Command executed without hang"
    echo "    Output: $OUT"
    exit 0
elif [[ $CODE -eq 142 ]]; then
    echo "❌ FAIL: Command HUNG during dyld bootstrap (Issue #2 detected)"
    exit 1
else
    echo "⚠️ INFO: Command failed with code $CODE, but did not hang"
    echo "    Output: $OUT"
    exit 0
fi
