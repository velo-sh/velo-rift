#!/bin/bash
# Test: Issue #6 - Synchronous Disk I/O in Async Daemon Handler
# Expected: FAIL (ManifestUpsert holds lock during blocking save())
# Fixed: SUCCESS (save() is async or deferred to a separate task)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: Synchronous Disk I/O in Daemon Handler ==="
echo "Issue: ManifestUpsert calls manifest.save() while holding MutexGuard, blocking async runtime."
echo ""

# Analyze daemon source
DAEMON_SRC="${PROJECT_ROOT}/crates/vrift-daemon/src/main.rs"

echo "[ANALYSIS] Checking ManifestUpsert handler..."

# Extract ManifestUpsert handler code
UPSERT_CODE=$(grep -A20 "ManifestUpsert" "$DAEMON_SRC" | head -25)

echo "$UPSERT_CODE"
echo ""

# Check for blocking patterns
if echo "$UPSERT_CODE" | grep -q "manifest.save"; then
    if echo "$UPSERT_CODE" | grep -q "\.await"; then
        # save is async
        if echo "$UPSERT_CODE" | grep -q "manifest.lock().await" && echo "$UPSERT_CODE" | grep -q "save.*\.await"; then
            echo "[WARN] manifest.save() might be async, but lock is held during I/O."
            EXIT_CODE=1
        else
            echo "[PASS] manifest.save() appears to be properly async."
            EXIT_CODE=0
        fi
    else
        echo "[FAIL] manifest.save() is synchronous and called while holding async MutexGuard!"
        echo ""
        echo "Performance Impact:"
        echo "  - All manifest updates are serialized"
        echo "  - Each save() blocks the async tokio runtime"
        echo "  - Under high concurrency, this creates massive latency"
        EXIT_CODE=1
    fi
else
    echo "[PASS] No synchronous save() pattern found."
    EXIT_CODE=0
fi

exit $EXIT_CODE
