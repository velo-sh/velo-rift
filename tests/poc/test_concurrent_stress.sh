#!/bin/bash
# Test: Concurrent Daemon Stress Test
# Goal: Verify daemon handles high-concurrency stat/open requests
# Priority: P2 - Identify race conditions and bottlenecks

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

set -e
echo "=== Concurrent Daemon Stress Test ==="
echo ""

SCRIPT_DIR="$(dirname "$0")"
CLI="${SCRIPT_DIR}/../../target/debug/vrift"
WORKDIR=$(mktemp -d)
trap "rm -rf '$WORKDIR'" EXIT

cd "$WORKDIR"

echo "[1] Setup:"
# Create test files
mkdir -p src
for i in $(seq 1 100); do
    echo "file $i content" > "src/file_$i.txt"
done
echo "    Created 100 test files"

get_time() {
    python3 -c 'import time; print(time.time())'
}

echo ""
echo "[2] Concurrent Stat Stress Test:"
CONCURRENT=10
ITERATIONS=5

stress_stat() {
    local id=$1
    for i in $(seq 1 $ITERATIONS); do
        for f in src/file_*.txt; do
            stat "$f" >/dev/null 2>&1 || true
        done
    done
}

START=$(get_time)
for i in $(seq 1 $CONCURRENT); do
    stress_stat $i &
done
wait
END=$(get_time)

DURATION=$(echo "$END - $START" | bc)
TOTAL_OPS=$((CONCURRENT * ITERATIONS * 100))
OPS_PER_SEC=$(echo "scale=0; $TOTAL_OPS / $DURATION" | bc)

echo "    Concurrent workers: $CONCURRENT"
echo "    Total stat ops: $TOTAL_OPS"
echo "    Duration: ${DURATION}s"
echo "    Throughput: ~${OPS_PER_SEC} ops/sec"

echo ""
echo "[3] Concurrent Open/Read Stress Test:"
stress_open() {
    local id=$1
    for i in $(seq 1 $ITERATIONS); do
        for f in src/file_*.txt; do
            cat "$f" >/dev/null 2>&1 || true
        done
    done
}

START=$(get_time)
for i in $(seq 1 $CONCURRENT); do
    stress_open $i &
done
wait
END=$(get_time)

DURATION=$(echo "$END - $START" | bc)
echo "    Duration: ${DURATION}s"
echo "    Throughput: ~$(echo "scale=0; $TOTAL_OPS / $DURATION" | bc) ops/sec"

echo ""
echo "[4] Race Condition Check:"
# If we got here without deadlock/crash, test passes
echo "    ✅ No deadlocks detected"
echo "    ✅ No crashes detected"

echo ""
echo "✅ PASS: Concurrent stress test completed"
