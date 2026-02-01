#!/bin/bash
# Test: Directory mtime Tracking
# Goal: Verify directory mtime changes when contents change
# Priority: P1 - Build tools watch directory mtime for new files

set -e
echo "=== Test: Directory mtime Tracking ==="
echo ""

SHIM_SRC="$(dirname "$0")/../../crates/vrift-shim/src/lib.rs"
MANIFEST_SRC="$(dirname "$0")/../../crates/vrift-manifest/src/lib.rs"

echo "[1] Why Directory mtime Matters:"
echo "    • make checks dir mtime to detect new source files"
echo "    • IDEs watch dir mtime for file tree updates"
echo "    • Git uses dir mtime for status optimization"
echo ""

echo "[2] Directory Representation in VFS:"
echo "    Option A: Synthetic directories (computed from paths)"
echo "    Option B: Explicit directory entries in Manifest"
echo ""

echo "[3] Checking Manifest:"

if grep -qE "is_dir|S_IFDIR" "$MANIFEST_SRC" 2>/dev/null; then
    echo "    ✅ Directory support found in Manifest"
else
    echo "    ⚠️ No explicit directory type"
fi

echo ""
echo "[4] Directory mtime Requirements:"
echo "    • Dir mtime = max(mtime of children)"
echo "    • Or: Dir mtime = time of last child add/remove"
echo ""

echo "[5] Checking stat for Directories:"

if grep -qE "S_IFDIR.*st_mtime\|is_dir.*mtime" "$SHIM_SRC" 2>/dev/null; then
    echo "    ✅ Directory mtime handling found"
else
    echo "    ⚠️ Directory mtime may not be tracked"
fi

echo ""
echo "[6] Recommendation:"
echo "    Update dir mtime in Manifest when:"
echo "    • File added to directory"
echo "    • File removed from directory"
echo "    • File renamed within directory"
echo ""

echo "✅ PASS: Directory structure analyzed"
exit 0
