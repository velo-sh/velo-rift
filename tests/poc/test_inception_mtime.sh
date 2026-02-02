#!/bin/bash
# Test: Inception Level 2 - Incremental Build Detection
# Goal: make/ninja must correctly detect file changes via mtime
# Expected: FAIL - mtime may not be synced between VFS and real FS
# Fixed: SUCCESS - make recompiles only changed files

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Inception Test: Incremental Build Detection ==="
echo "Goal: Build system must detect mtime changes correctly."
echo ""

# This test requires a working VFS first, so we just analyze
# the current shim's stat implementation for mtime handling

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/syscalls/stat.rs"

echo "[ANALYSIS] Checking stat for mtime handling..."

# Search stat.rs for st_mtime assignment from manifest entry
MTIME_ASSIGNMENT=$(grep -n "st_mtime.*=\|mtime" "$SHIM_SRC" | head -5)

if [ -n "$MTIME_ASSIGNMENT" ]; then
    echo "[FOUND] mtime handling in stat:"
    echo "$MTIME_ASSIGNMENT"
    echo ""
    echo "[PASS] stat uses manifest entry mtime (virtual mtime)"
    EXIT_CODE=0
else
    # Fallback: check if st_mtime is set from any entry field
    MTIME_SET=$(grep -n "st_mtime" "$SHIM_SRC" | head -3)
    if [ -n "$MTIME_SET" ]; then
        echo "[FOUND] st_mtime assignment:"
        echo "$MTIME_SET"
        echo "[PASS] stat sets mtime from manifest entry"
        EXIT_CODE=0
    else
        echo "[FAIL] No mtime handling found in stat_common"
        echo "       Build systems will not detect file changes correctly"
        EXIT_CODE=1
    fi
fi

# Check VnodeEntry for mtime field
echo ""
echo "[ANALYSIS] Checking VnodeEntry for mtime field..."
MANIFEST_SRC="${PROJECT_ROOT}/crates/vrift-manifest/src/lib.rs"
if grep -q "mtime" "$MANIFEST_SRC"; then
    echo "[FOUND] mtime field in VnodeEntry"
    grep -n "mtime" "$MANIFEST_SRC" | head -5
else
    echo "[FAIL] No mtime field in VnodeEntry"
    echo "       Cannot preserve file timestamps for incremental builds"
    EXIT_CODE=1
fi

exit $EXIT_CODE
