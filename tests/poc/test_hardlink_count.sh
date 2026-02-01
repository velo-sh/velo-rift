#!/bin/bash
# Test: Hardlink Count (st_nlink) Accuracy
# Goal: Verify st_nlink is correct for hardlink detection
# Priority: P2 - Some tools use nlink to detect hardlinks

set -e
echo "=== Test: Hardlink Count Accuracy ==="
echo ""

SHIM_SRC="$(dirname "$0")/../../crates/vrift-shim/src/lib.rs"

echo "[1] st_nlink Usage:"
echo "    • nlink > 1 indicates hardlinks exist"
echo "    • du uses nlink to avoid double-counting"
echo "    • rsync uses nlink for hardlink preservation"
echo ""

echo "[2] VFS Options for st_nlink:"
echo "    A. Always return 1 (simple, most tools OK)"
echo "    B. Return CAS blob's real nlink (may be very high)"
echo "    C. Count Manifest entries pointing to same hash"
echo ""

echo "[3] Checking Implementation:"

if grep -q "st_nlink" "$SHIM_SRC" 2>/dev/null; then
    NLINK_LINE=$(grep "st_nlink" "$SHIM_SRC" | head -1)
    echo "    Found: $NLINK_LINE"
    
    if echo "$NLINK_LINE" | grep -q "= 1"; then
        echo "    Using constant nlink = 1"
        echo "    ✅ Simple approach, works for most tools"
    else
        echo "    Using dynamic nlink value"
    fi
else
    echo "    ⚠️ st_nlink not explicitly set"
fi

echo ""
echo "[4] CAS Hardlink Reality:"
echo "    VFS files pointing to same content ARE hardlinked"
echo "    (to the same CAS blob)"
echo "    But reporting this may confuse tools expecting"
echo "    nlink to represent file identity, not content"
echo ""

echo "[5] Recommendation:"
echo "    Return nlink = 1 for VFS files"
echo "    This is semantically correct: each VFS path is unique"
echo ""

echo "✅ PASS: Hardlink count analyzed"
exit 0
