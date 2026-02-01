#!/bin/bash
# Test: Issue #7 - Incomplete LMDB Transition (Architectural Regression)
# Expected: FAIL (Daemon uses legacy Bincode Manifest instead of LMDB)
# Fixed: SUCCESS (Daemon uses LmdbManifest for O(1) concurrent reads)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: Incomplete LMDB Transition ==="
echo "Issue: CLI uses LmdbManifest, but Daemon still uses legacy Bincode Manifest."
echo ""

DAEMON_SRC="${PROJECT_ROOT}/crates/vrift-daemon/src/main.rs"

echo "[ANALYSIS] Checking daemon's manifest type..."

# Check what manifest type daemon uses
if grep -q "LmdbManifest" "$DAEMON_SRC"; then
    echo "[PASS] Daemon uses LmdbManifest."
    EXIT_CODE=0
else
    echo "[FAIL] Daemon does NOT use LmdbManifest."
    echo ""
    
    # Show what it does use
    echo "Current manifest usage in daemon:"
    grep -n "Manifest" "$DAEMON_SRC" | head -10
    
    echo ""
    echo "Impact:"
    echo "  - CLI writes to LMDB, Daemon reads from Bincode"
    echo "  - Data is not shared between CLI and Daemon"
    echo "  - LMDB benefits (ACID, O(1) reads) are not utilized at runtime"
    EXIT_CODE=1
fi

exit $EXIT_CODE
