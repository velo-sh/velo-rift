#!/bin/bash
# Test: Inception Level 3 - Linker Object File Identity
# Goal: ld/lld must see consistent inode/size for object files
# Expected: FAIL - fstat is passthrough, returns CAS blob identity
# Fixed: SUCCESS - fstat returns virtual file identity

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Inception Test: Linker Object File Identity ==="
echo "Goal: Linker must see consistent virtual file identity via fstat."
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[ANALYSIS] Checking fstat_impl for inode handling..."

FSTAT_IMPL=$(grep -A10 "unsafe fn fstat_impl" "$SHIM_SRC" | head -15)

echo "$FSTAT_IMPL"
echo ""

# Check if fstat returns virtual metadata
if echo "$FSTAT_IMPL" | grep -q "pass through\|passthrough\|// For fstat"; then
    echo "[FAIL] fstat_impl is a passthrough!"
    echo ""
    echo "Impact on linker:"
    echo "  - ld caches object files by (dev, inode) pair"
    echo "  - If VFS returns CAS blob identity, linker may:"
    echo "    1. Use stale cached object code"
    echo "    2. Fail to detect duplicate objects"
    echo "    3. Produce corrupt binaries"
    EXIT_CODE=1
else
    echo "[PASS] fstat_impl appears to handle virtual metadata"
    EXIT_CODE=0
fi

# Check if open_fds tracking exists for fd -> path mapping
echo ""
echo "[ANALYSIS] Checking for FD -> Path mapping..."
if grep -q "open_fds\|fd_table\|FdMapping" "$SHIM_SRC"; then
    echo "[FOUND] FD tracking mechanism exists"
    grep -n "open_fds\|fd_table" "$SHIM_SRC" | head -5
else
    echo "[FAIL] No FD tracking - cannot map fstat(fd) to virtual path"
fi

exit $EXIT_CODE
