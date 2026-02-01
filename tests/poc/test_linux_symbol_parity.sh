#!/bin/bash
# Test: Linux Symbol Parity (Missing 16 Shims)
# Goal: Detect missing syscall interceptions on Linux architecture.

set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SHIM_PATH="${PROJECT_ROOT}/target/debug/libvelo_shim.so"

echo "=== Test: Linux Symbol Parity ==="

if [[ "$(uname)" == "Darwin" ]]; then
    echo "⚠️  This test is designed for Linux analysis. Skipping on macOS."
    exit 0
fi

# List of shims required for high-parity Linux support
REQUIRED_SYMBOLS=(
    "openat"
    "faccessat"
    "fstatat"
    "opendir"
    "readdir"
    "closedir"
    "readlink"
    "posix_spawn"
    "posix_spawnp"
    "mmap"
    "munmap"
    "dlopen"
    "dlsym"
    "access"
    "read"
    "fcntl"
)

MISSING_COUNT=0
for sym in "${REQUIRED_SYMBOLS[@]}"; do
    if nm -gD "$SHIM_PATH" 2>/dev/null | grep -q " ${sym}$"; then
        echo "✅ FOUND: $sym"
    else
        echo "❌ MISSING: $sym"
        MISSING_COUNT=$((MISSING_COUNT + 1))
    fi
done

if [[ $MISSING_COUNT -gt 0 ]]; then
    echo ""
    echo "❌ FAIL: Linux shim is missing $MISSING_COUNT critical interceptions."
    exit 1
else
    echo ""
    echo "✅ PASS: All required Linux shims detected."
fi
