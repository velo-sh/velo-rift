#!/bin/bash
# Test: Issue #8 - Blocking I/O in Shim's close() Interception
# Expected: FAIL (close() takes > 1 second for large file)
# Fixed: SUCCESS (close() returns immediately, async ingest in background)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: Blocking I/O in Shim close() ==="
echo "Issue: Shim performs std::fs::read + hash computation INSIDE close(), blocking the caller."
echo ""

# Check source code for blocking pattern
echo "[ANALYSIS] Checking close_impl for blocking I/O..."

# Look for fs::read or fs::metadata inside close_impl
CLOSE_IMPL=$(grep -A50 "unsafe fn close_impl" "${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs" | head -60)

echo "$CLOSE_IMPL"
echo ""

if echo "$CLOSE_IMPL" | grep -q "std::fs::read"; then
    echo "[FAIL] close_impl contains std::fs::read - blocking I/O detected!"
    echo "       This will cause hangs when closing large virtual files."
    EXIT_CODE=1
elif echo "$CLOSE_IMPL" | grep -q "CasStore::compute_hash"; then
    echo "[FAIL] close_impl contains CasStore::compute_hash - CPU-bound blocking detected!"
    echo "       This will cause hangs for large files."
    EXIT_CODE=1
else
    echo "[PASS] close_impl does not appear to have blocking I/O."
    EXIT_CODE=0
fi

exit $EXIT_CODE
