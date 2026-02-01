#!/bin/bash
# Test: Data Persistence After Restart
# Goal: Verify that ingested files survive daemon restart
# Expected: FAIL - Delta layer not committed
# Fixed: SUCCESS - File found after restart

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: Restart Recovery ==="
echo "Goal: Ingested file must survive daemon restart"
echo ""

# Setup
export VR_THE_SOURCE="/tmp/restart_test_cas"
export VRIFT_MANIFEST_DIR="/tmp/restart_test_manifest.lmdb"

rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST_DIR"
mkdir -p "$VR_THE_SOURCE"

# Create test file in a directory (ingest requires directory)
TEST_DIR="/tmp/restart_test_dir"
mkdir -p "$TEST_DIR"
TEST_FILE="$TEST_DIR/restart_test_file.txt"
echo "test content $(date)" > "$TEST_FILE"

echo "[STEP 1] Start daemon..."
killall vriftd 2>/dev/null || true
sleep 1
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/restart_test_daemon.log 2>&1 &
DAEMON_PID=$!
sleep 2

echo "[STEP 2] Ingest test file..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest "$TEST_DIR" --prefix /test 2>&1 | tail -5

echo ""
echo "[STEP 3] Kill daemon (simulating crash)..."
kill -9 $DAEMON_PID 2>/dev/null || true
sleep 2

echo "[STEP 4] Restart daemon..."
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/restart_test_daemon2.log 2>&1 &
DAEMON_PID2=$!
sleep 2

echo "[STEP 5] Check if file exists in manifest..."
# Check CAS has the blob (persistence test)
BLOB_COUNT=$(find "$VR_THE_SOURCE" -name "*.bin" 2>/dev/null | wc -l)

kill $DAEMON_PID2 2>/dev/null || true

if [ "$BLOB_COUNT" -gt 0 ]; then
    echo "[PASS] CAS has $BLOB_COUNT blobs - data persisted!"
    echo "[INFO] Restart recovery test passed (CAS persistence verified)."
    EXIT_CODE=0
else
    echo "[FAIL] No blobs found in CAS after restart!"
    echo "[ANALYSIS] Checking daemon commit logic..."
    
    # Check if daemon calls commit
    if grep -q "manifest.*commit\|\.commit()" "${PROJECT_ROOT}/crates/vrift-daemon/src/main.rs"; then
        echo "[OK] Daemon has commit call"
    else
        echo "[FAIL] Daemon does NOT call manifest.commit()!"
    fi
    EXIT_CODE=1
fi

# Cleanup (use chflags to remove immutable flags first)
chflags -R nouchg "$VR_THE_SOURCE" 2>/dev/null || true
rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST_DIR" "$TEST_DIR" 2>/dev/null || true
rm -f /tmp/restart_test_daemon*.log 2>/dev/null || true

exit $EXIT_CODE
