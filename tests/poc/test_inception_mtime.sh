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

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[ANALYSIS] Checking stat_common for mtime handling..."

# Extract stat_common function
STAT_IMPL=$(grep -A30 "unsafe fn stat_common" "$SHIM_SRC" | head -40)

echo "$STAT_IMPL"
echo ""

# Check if mtime is being set from manifest entry
if echo "$STAT_IMPL" | grep -q "st_mtime\|mtime"; then
    echo "[FOUND] mtime handling in stat_common"
    
    # Check if it uses manifest entry mtime or real file mtime
    if echo "$STAT_IMPL" | grep -q "entry.*mtime\|entry\.mtime"; then
        echo "[PASS] stat uses manifest entry mtime (virtual mtime)"
        EXIT_CODE=0
    else
        echo "[WARN] stat may not use manifest mtime correctly"
        EXIT_CODE=1
    fi
else
    echo "[FAIL] No mtime handling found in stat_common"
    echo "       Build systems will not detect file changes correctly"
    EXIT_CODE=1
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
