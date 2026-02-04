#!/bin/bash
# Proof of Failure: openat Relative Path Resolution Gap
# Demonstrates that 'openat' with relative paths can bypass VFS detection
# because the shim incorrectly resolves relative paths against the project root
# instead of the provided dirfd.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

# 1. Setup VFS territory
mkdir -p "$WORK_DIR/vfs/dir"
echo "VFS SECRET" > "$WORK_DIR/vfs/dir/secret.txt"

# 2. Compile Test Program
gcc -o "$WORK_DIR/openat_test" "$SCRIPT_DIR/openat_test.c"

# 3. VRift Init
export VRIFT_VFS_PREFIX="$WORK_DIR/vfs"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

echo "🧪 Case 1: Standard Absolute Path (Should Hit VFS)"
"$WORK_DIR/openat_test" "/" "$WORK_DIR/vfs/dir/secret.txt" | grep "VFS SECRET" || echo "❌ Direct hit failed"

echo "🧪 Case 2: Relative path from non-VFS parent (Should Hit VFS if resolved correctly)"
# Pre-condition: cd to /tmp, open /tmp as dirfd, then openat(dirfd, "$WORK_DIR/vfs/dir/secret.txt")
# Actually, let's use a sibling directory.
mkdir -p "$WORK_DIR/real"
cd "$WORK_DIR/real"

# The path to secret.txt from $WORK_DIR/real is ../vfs/dir/secret.txt
REL_PATH="../vfs/dir/secret.txt"

echo "   Running: openat(\"$WORK_DIR/real\", \"$REL_PATH\")"
OUT=$("$WORK_DIR/openat_test" "$WORK_DIR/real" "$REL_PATH" 2>&1)

if echo "$OUT" | grep -q "VFS SECRET"; then
    echo "✅ Success: Shim correctly identified VFS path even when relative."
else
    echo "❌ Failure: Shim MISSED VFS path via relative openat."
    echo "   Output: $OUT"
    echo "💥 SLAP: This is a VFS escape vector."
fi
