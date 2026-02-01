#!/bin/bash
# Test: Issue #2 - TLS Bootstrap Hang (_tlv_bootstrap)
# Expected: FAIL (shim uses thread_local! which hangs during early bootstrap)
# Fixed: SUCCESS (shim uses pthread_key_t or atomic guards)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: TLS Bootstrap Hang ==="
echo "Issue: Shim uses thread_local! which can hang during macOS dyld bootstrap."
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[ANALYSIS] Checking shim's recursion guard implementation..."

# Check for thread_local! usage
if grep -q "thread_local!" "$SHIM_SRC"; then
    echo "[FAIL] Shim uses thread_local! macro."
    echo ""
    echo "Risk:"
    echo "  - macOS dyld may call intercepted functions before TLS is initialized"
    echo "  - Accessing thread_local! at this stage causes _tlv_bootstrap deadlock"
    echo ""
    echo "Found usage:"
    grep -n "thread_local!" "$SHIM_SRC"
    EXIT_CODE=1
elif grep -q "pthread_key_t\|pthread_getspecific" "$SHIM_SRC"; then
    echo "[PASS] Shim uses pthread_key_t (bootstrap-safe)."
    EXIT_CODE=0
elif grep -q "AtomicBool\|AtomicUsize" "$SHIM_SRC" && grep -q "RECURSION\|IN_SHIM" "$SHIM_SRC"; then
    echo "[PASS] Shim uses atomic-based recursion guard."
    EXIT_CODE=0
else
    echo "[WARN] Could not determine recursion guard strategy."
    EXIT_CODE=0
fi

exit $EXIT_CODE
