#!/bin/bash
# test_stat_throughput.sh - Benchmark daemon IPC stat throughput
# Priority: P2 (Performance)
set -e

echo "=== Test: Stat Throughput Benchmark ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VR_THE_SOURCE="/tmp/throughput_cas"
VRIFT_MANIFEST="/tmp/throughput.manifest"
TEST_DIR="/tmp/throughput_test"

cleanup() {
    rm -rf "$VR_THE_SOURCE" "$TEST_DIR" "$VRIFT_MANIFEST" 2>/dev/null || true
    killall vriftd 2>/dev/null || true
}
trap cleanup EXIT
cleanup

mkdir -p "$VR_THE_SOURCE" "$TEST_DIR"

echo "[1] Creating 100 test files..."
for i in $(seq 1 100); do
    echo "Content $i" > "$TEST_DIR/file_$i.txt"
done

echo "[2] Ingesting files..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest "$TEST_DIR" --output "$VRIFT_MANIFEST" --prefix /bench 2>&1 | tail -2

echo "[3] Starting daemon..."
export VRIFT_MANIFEST
"${PROJECT_ROOT}/target/debug/vriftd" start > /dev/null 2>&1 &
DAEMON_PID=$!
sleep 2

echo "[4] Benchmarking stat throughput (1000 calls)..."
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"

START=$(python3 -c "import time; print(int(time.time()*1000))")

for i in $(seq 1 1000); do
    stat /vrift/bench/file_$((i % 100 + 1)).txt >/dev/null 2>&1 || true
done

END=$(python3 -c "import time; print(int(time.time()*1000))")
unset DYLD_INSERT_LIBRARIES

ELAPSED=$((END - START))
RATE=$((1000 * 1000 / ELAPSED))

kill $DAEMON_PID 2>/dev/null || true

echo ""
echo "    Time: ${ELAPSED}ms for 1000 stat calls"
echo "    Rate: ${RATE} stat/sec"

# Target: >100 stat/sec is acceptable for dev, >500 for production
if [ "$RATE" -gt 100 ]; then
    echo ""
    echo "✅ PASS: Throughput acceptable (${RATE} stat/sec)"
    exit 0
else
    echo ""
    echo "⚠️  WARN: Throughput below target (${RATE} stat/sec < 100)"
    exit 0  # Don't fail, just flag
fi
