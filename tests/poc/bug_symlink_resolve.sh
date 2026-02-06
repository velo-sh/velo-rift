#!/bin/bash
# Proof of Failure: Symlink Virtualization Gap
# Demonstrates that virtual symlinks might resolve to host paths or fail.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

# 1. Setup VFS territory
mkdir -p "$WORK_DIR/vfs"
echo "TARGET CONTENT" > "$WORK_DIR/vfs/target.txt"
ln -s "target.txt" "$WORK_DIR/vfs/link.txt"

# 2. VRift Inception
export VRIFT_VFS_PREFIX="$WORK_DIR/vfs"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

# Use local 'ls' or 'readlink' to bypass SIP
cp /usr/bin/readlink "$WORK_DIR/readlink"

echo "🧪 Case: readlink on virtual symlink"
OUT=$("$WORK_DIR/readlink" "$WORK_DIR/vfs/link.txt")

echo "   Result: $OUT"

if [ "$OUT" == "target.txt" ]; then
    echo "✅ Success: Symlink resolved to relative virtual path."
else
    echo "❌ Failure: Symlink resolved to unexpected path: $OUT"
    echo "💥 SLAP: Symlink virtualization is broken."
fi

echo "🧪 Case: stat -L (follow link)"
cp /usr/bin/stat "$WORK_DIR/stat"
OUT=$("$WORK_DIR/stat" -L "$WORK_DIR/vfs/link.txt")

if echo "$OUT" | grep -q "TARGET CONTENT"; then
     # stat -L usually shows metadata, but we can cat it
     echo "✅ stat -L seems to follow"
fi

if cat "$WORK_DIR/vfs/link.txt" | grep -q "TARGET CONTENT"; then
    echo "✅ Success: cat followed virtual symlink."
else
    echo "❌ Failure: cat failed to follow virtual symlink."
fi
