#!/bin/bash
# Proof of Failure: Rename POSIX Compliance Gap
# Demonstrates that returning EPERM instead of EXDEV for cross-boundary renames
# breaks the standard 'mv' utility's fallback mechanism.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

# Setup
mkdir -p "$WORK_DIR/external"
mkdir -p "$WORK_DIR/vfs_territory"
echo "test data" > "$WORK_DIR/external/source.txt"

# VRIFT Environment
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
if [ ! -f "$SHIM_LIB" ]; then
    echo "❌ Shim not found at $SHIM_LIB. Build it first."
    exit 1
fi

# 2. Compile Test Tool
gcc -o "$WORK_DIR/rename_test" "$SCRIPT_DIR/rename_test.c"

# VRIFT Environment
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
if [ ! -f "$SHIM_LIB" ]; then
    echo "❌ Shim not found at $SHIM_LIB. Build it first."
    exit 1
fi

export VRIFT_VFS_PREFIX="$WORK_DIR/vfs_territory"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

echo "🧪 Test: Cross-boundary rename (External -> VFS)"
echo "   Command: rename_test $WORK_DIR/external/source.txt $WORK_DIR/vfs_territory/target.txt"

# This should fail if the shim returns EPERM, but succeed (via copy) if it returns EXDEV
OUT=$("$WORK_DIR/rename_test" "$WORK_DIR/external/source.txt" "$WORK_DIR/vfs_territory/target.txt" 2>&1)

if echo "$OUT" | grep -q "Success"; then
    echo "✅ Success: rename succeeded (likely via copy fallback or direct link if on same FS)."
    echo "   Wait, rename doesn't have a fallback. If it succeeded, it means it bypassed the shim or the shim returned 0."
    exit 1
else
    echo "❌ Failure as expected (rename doesn't fallback, but we check ERRNO)"
    echo "   Output: $OUT"
    
    if echo "$OUT" | grep -q "Operation not permitted"; then
        echo "💥 SLAP: The shim returned EPERM (Operation not permitted)."
        echo "   POSIX compliance requires EXDEV (Cross-device link) to trigger mv's copy fallback."
    elif echo "$OUT" | grep -q "Cross-device link"; then
        echo "✅ Good: The shim returned EXDEV (Cross-device link)."
    else
        echo "❓ Unknown error. Check results."
    fi
fi
