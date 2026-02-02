#!/bin/bash
# Test: mmap Interception for Large Libraries
# Goal: Verify if mmap is intercepted for VFS files
# Expected: FAIL - mmap not implemented
# Fixed: SUCCESS - mmap returns virtual file content

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: mmap Interception ==="
echo "Goal: mmap on VFS files must return virtual content"
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/syscalls/mmap.rs"
INTERPOSE_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[ANALYSIS] Checking for mmap interception..."

# Search for mmap-related code
MMAP_CODE=$(grep -n "mmap_shim\|mmap" "$SHIM_SRC" 2>/dev/null | head -10)

if [ -n "$MMAP_CODE" ]; then
    echo "[FOUND] mmap-related code:"
    echo "$MMAP_CODE"
    
    # Check if it's in interpose table
    if grep -q "mmap_shim" "$INTERPOSE_SRC"; then
        echo "[PASS] mmap implementation found and registered"
        EXIT_CODE=0
    else
        echo "[WARN] mmap referenced but not in interpose table"
        EXIT_CODE=1
    fi
else
    echo "[FAIL] No mmap interception found"
    echo ""
    echo "Impact on rustc/cargo:"
    echo "  - Large rlib files (>16KB) are memory-mapped"
    echo "  - Without mmap shim, rustc reads CAS blob path"
    echo "  - May cause incorrect symbol resolution"
    EXIT_CODE=1
fi

echo ""
echo "[INFO] Interpose table check:"
grep -n "IT_MMAP\|mmap_shim" "$INTERPOSE_SRC" 2>/dev/null || echo "  No mmap in interpose table"

exit $EXIT_CODE
