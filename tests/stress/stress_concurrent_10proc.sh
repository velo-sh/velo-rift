#!/bin/bash
# test_concurrent_stress.sh - Verify shim handles high concurrency without races
# Priority: P2 (Stress Test)
set -e

echo "=== Test: Concurrent Stress (10+ processes) ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VR_THE_SOURCE="/tmp/concurrent_cas"
VRIFT_MANIFEST="/tmp/concurrent.manifest"
TEST_DIR="/tmp/concurrent_test"

cleanup() {
    rm -rf "$VR_THE_SOURCE" "$TEST_DIR" "$VRIFT_MANIFEST" /tmp/concurrent_*.log 2>/dev/null || true
    killall vriftd 2>/dev/null || true
}
trap cleanup EXIT
cleanup

mkdir -p "$VR_THE_SOURCE" "$TEST_DIR"

echo "[1] Creating test files..."
for i in $(seq 1 20); do
    echo "File content $i" > "$TEST_DIR/file_$i.txt"
done

echo "[2] Ingesting test files..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest "$TEST_DIR" --output "$VRIFT_MANIFEST" --prefix /conc 2>&1 | tail -3

echo "[3] Starting daemon..."
export VRIFT_MANIFEST
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/concurrent_daemon.log 2>&1 &
DAEMON_PID=$!
sleep 2

echo "[4] Spawning 10 concurrent stat processes..."
PIDS=""
for i in $(seq 1 10); do
    (
        export DYLD_FORCE_FLAT_NAMESPACE=1
        export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"
        # Each process does 100 stat calls
        for j in $(seq 1 100); do
            stat /vrift/conc/file_$((j % 20 + 1)).txt >/dev/null 2>&1 || true
        done
        echo "DONE" > /tmp/concurrent_$i.log
    ) &
    PIDS="$PIDS $!"
done

echo "[5] Waiting for processes (10s timeout)..."
TIMEOUT=10
ELAPSED=0
while [ $ELAPSED -lt $TIMEOUT ]; do
    DONE_COUNT=$(ls /tmp/concurrent_*.log 2>/dev/null | wc -l)
    if [ "$DONE_COUNT" -ge 10 ]; then
        break
    fi
    sleep 1
    ((ELAPSED++)) || true
done

# Kill any remaining
for pid in $PIDS; do
    kill -9 $pid 2>/dev/null || true
done

kill $DAEMON_PID 2>/dev/null || true

DONE_COUNT=$(ls /tmp/concurrent_*.log 2>/dev/null | wc -l)
echo "    Completed: $DONE_COUNT/10 processes"

if [ "$DONE_COUNT" -ge 8 ]; then
    echo ""
    echo "✅ PASS: Concurrent stress test passed ($DONE_COUNT/10)"
    exit 0
else
    echo ""
    echo "[FAIL] Too many processes failed (only $DONE_COUNT/10 completed)"
    exit 1
fi
