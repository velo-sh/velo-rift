#!/bin/bash
# Test: Virtual Directory Listing for cargo
# Goal: opendir/readdir must list virtual directory contents
# Expected: FAIL - opendir_impl is passthrough
# Fixed: SUCCESS - Virtual directory contents returned

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: Virtual Directory Listing ==="
echo "Goal: ls /vrift/project/src/ must work"
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[ANALYSIS] opendir_impl implementation:"
OPENDIR_IMPL=$(grep -A5 "unsafe fn opendir_impl" "$SHIM_SRC")
echo "$OPENDIR_IMPL"
echo ""

# Check if opendir has virtual path handling
if echo "$OPENDIR_IMPL" | grep -q "vfs_prefix\|starts_with\|virtual"; then
    echo "[PASS] opendir_impl has virtual path handling"
    EXIT_CODE=0
else
    echo "[FAIL] opendir_impl is a passthrough - no virtual directory support"
    echo ""
    echo "Impact on cargo:"
    echo "  - Cannot discover source files in /vrift/project/src/"
    echo "  - Cannot list dependencies in /vrift/project/target/"
    echo "  - cargo build will fail with 'cannot find crate'"
    EXIT_CODE=1
fi

echo ""
echo "[ANALYSIS] readdir_impl implementation:"
# Check if readdir exists and handles virtual directories
READDIR_IMPL=$(grep -A10 "unsafe fn readdir_impl\|fn readdir_shim" "$SHIM_SRC" | head -15)
if [ -n "$READDIR_IMPL" ]; then
    echo "$READDIR_IMPL"
    if echo "$READDIR_IMPL" | grep -q "virtual\|synthetic\|state"; then
        echo "[PASS] readdir has virtual handling"
    else
        echo "[WARN] readdir exists but may not handle virtual directories"
    fi
else
    echo "[FAIL] No readdir_impl found"
fi

exit $EXIT_CODE
