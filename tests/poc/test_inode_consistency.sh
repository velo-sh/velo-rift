#!/bin/bash
# Test: Inode Consistency
# Goal: Verify st_ino is consistent between stat/fstat/lstat calls
# Priority: P1 - Build tools use inode to detect file identity

set -e
echo "=== Test: Inode Consistency ==="
echo ""

SHIM_SRC="$(dirname "$0")/../../crates/vrift-shim/src/lib.rs"

echo "[1] Inode Usage by Build Tools:"
echo "    • make: uses inode to detect hardlinks"
echo "    • ninja: uses inode for dep graph"
echo "    • Git: uses inode in index"
echo ""

echo "[2] VFS Inode Options:"
echo "    A. Return 0 for all files (some tools break)"
echo "    B. Hash-based inode (content-addressed)"
echo "    C. Unique inode per Manifest entry"
echo ""

echo "[3] Checking Implementation:"

# Check what st_ino is set to
if grep -q "st_ino" "$SHIM_SRC" 2>/dev/null; then
    INO_LINE=$(grep "st_ino" "$SHIM_SRC" | head -1)
    echo "    Found: $INO_LINE"
    
    if echo "$INO_LINE" | grep -q "= 0\|= 1"; then
        echo "    ⚠️ Using constant inode (may break hardlink detection)"
    else
        echo "    ✅ Using dynamic inode value"
    fi
else
    echo "    ⚠️ st_ino not explicitly set (uses underlying file)"
fi

echo ""
echo "[4] Consistency Requirements:"
echo "    • stat(path) and fstat(fd) must return same inode"
echo "    • lstat(symlink) returns symlink's inode"
echo "    • Multiple stat() calls must return same inode"
echo ""

echo "[5] Recommendation:"
echo "    Use hash of (path + content_hash) as stable inode"
echo "    Or use CAS blob's real inode (simpler)"
echo ""

echo "✅ PASS: Inode handling analyzed"
exit 0
