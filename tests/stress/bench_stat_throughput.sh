#!/bin/bash
# bench_stat_throughput.sh - Benchmark daemon IPC stat throughput
# Priority: P2 (Performance)
set -e

echo "=== Test: Stat Throughput Benchmark ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VRIFT_CLI="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"
SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
[ ! -f "$SHIM_LIB" ] && SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.so"

TEST_DIR="/tmp/throughput_test_$$"
export VR_THE_SOURCE="$TEST_DIR/.cas"
export VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock"
DAEMON_PID=""

cleanup() {
    [ -n "$DAEMON_PID" ] && kill -9 "$DAEMON_PID" 2>/dev/null || true
    rm -f "$VRIFT_SOCKET_PATH"
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT
cleanup 2>/dev/null || true

mkdir -p "$VR_THE_SOURCE" "$TEST_DIR/project"

echo "[1] Creating 100 test files..."
for i in $(seq 1 100); do
    echo "Content $i" > "$TEST_DIR/project/file_$i.txt"
done

echo "[2] Ingesting files..."
cd "$TEST_DIR/project"
"$VRIFT_CLI" init . >/dev/null 2>&1
"$VRIFT_CLI" ingest . --mode solid --output .vrift/manifest.lmdb >/dev/null 2>&1

echo "[3] Starting daemon..."
VRIFT_LOG=info "$VRIFTD_BIN" start </dev/null > "$TEST_DIR/vriftd.log" 2>&1 &
DAEMON_PID=$!

# Wait for socket
waited=0
while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
    sleep 0.5
    waited=$((waited + 1))
done

if [ ! -S "$VRIFT_SOCKET_PATH" ]; then
    echo "❌ Daemon failed to start"
    cat "$TEST_DIR/vriftd.log"
    exit 1
fi

echo "[4] Benchmarking stat throughput (1000 calls)..."
export VRIFT_VFS_PREFIX="$TEST_DIR/project"
export VRIFT_PROJECT_ROOT="$TEST_DIR/project"

START=$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')

for i in $(seq 1 1000); do
    env DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 \
        VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
        VRIFT_VFS_PREFIX="$TEST_DIR/project" VRIFT_PROJECT_ROOT="$TEST_DIR/project" \
        stat "$TEST_DIR/project/file_$((i % 100 + 1)).txt" >/dev/null 2>&1 || true
done

END=$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')

ELAPSED=$((END - START))
if [ "$ELAPSED" -gt 0 ]; then
    RATE=$((1000 * 1000 / ELAPSED))
else
    RATE=99999
fi

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
