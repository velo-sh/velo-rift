#!/bin/bash
# Simple Stat Benchmark: VFS vs Real FS
# Aligned with test_shim_basic.sh environment setup

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Use release binaries
VRIFT_BIN="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"
SHIM_PATH="${PROJECT_ROOT}/target/release/libvrift_shim.dylib"

if [ ! -f "$VRIFT_BIN" ] || [ ! -f "$VRIFTD_BIN" ] || [ ! -f "$SHIM_PATH" ]; then
    echo "❌ Release binaries not found. Run: cargo build --release"
    exit 1
fi

echo "=== Simple Stat Benchmark ==="
echo ""

# Setup temp workspace
TEST_DIR=$(mktemp -d)
SOURCE_DIR="$TEST_DIR/source"
CAS_ROOT="$TEST_DIR/cas"

cleanup() {
    pkill vriftd 2>/dev/null || true
    chmod -R +w "$TEST_DIR" 2>/dev/null || true
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$SOURCE_DIR" "$CAS_ROOT"

# Create test files (1000 files)
echo "[1/4] Creating 1000 test files..."
for i in $(seq 1 1000); do
    echo "test data $i" > "$SOURCE_DIR/file_$i.txt"
done

# Ingest with --prefix "" (like test_shim_basic.sh)
echo "[2/4] Ingesting to VFS..."
export VR_THE_SOURCE="$CAS_ROOT"
"$VRIFT_BIN" ingest "$SOURCE_DIR" --prefix "" -o "$SOURCE_DIR/vrift.manifest" > /dev/null 2>&1

# Start daemon (like test_shim_basic.sh)
echo "[3/4] Starting vriftd..."
pkill vriftd 2>/dev/null || true
sleep 1

export VR_THE_SOURCE="$CAS_ROOT"
export RUST_LOG=info
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
VRIFTD_PID=$!
sleep 2

# Behavior-based daemon verification instead of pgrep
if ! "$VRIFT_BIN" daemon status 2>/dev/null | grep -q "running\|Operational"; then
    # Fallback: check socket exists
    if [ ! -S "/tmp/vrift.sock" ]; then
        echo "❌ ERROR: Daemon not running (behavior check failed)."
        cat "$TEST_DIR/daemon.log"
        exit 1
    fi
fi

echo "✅ VFS ready (verified via behavior check)"

# Create C benchmark for /vrift paths
cat > "$TEST_DIR/bench.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <time.h>

long long now_ns() {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int main() {
    struct stat sb;
    long long start = now_ns();
    
    for (int i = 1; i <= 1000; i++) {
        char path[256];
        snprintf(path, sizeof(path), "/vrift/file_%d.txt", i);
        if (lstat(path, &sb) != 0) {
            printf("lstat failed for file_%d.txt\n", i);
            return 1;
        }
    }
    
    long long end = now_ns();
    double ms = (end - start) / 1000000.0;
    printf("Time: %.2f ms, Avg: %.2f µs/file\n", ms, ms * 1000 / 1000);
    return 0;
}
EOF

# Create C benchmark for real FS paths
cat > "$TEST_DIR/bench_fs.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <time.h>

long long now_ns() {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int main() {
    struct stat sb;
    long long start = now_ns();
    
    for (int i = 1; i <= 1000; i++) {
        char path[256];
        snprintf(path, sizeof(path), "file_%d.txt", i);
        if (lstat(path, &sb) != 0) {
            printf("lstat failed for file_%d.txt\n", i);
            return 1;
        }
    }
    
    long long end = now_ns();
    double ms = (end - start) / 1000000.0;
    printf("Time: %.2f ms, Avg: %.2f µs/file\n", ms, ms * 1000 / 1000);
    return 0;
}
EOF

cc -O2 -o "$TEST_DIR/bench" "$TEST_DIR/bench.c"
cc -O2 -o "$TEST_DIR/bench_fs" "$TEST_DIR/bench_fs.c"
codesign -f -s - "$TEST_DIR/bench" 2>/dev/null || true
codesign -f -s - "$TEST_DIR/bench_fs" 2>/dev/null || true

# Test 1: Real FS (from source dir, no shim)
echo ""
echo "[4/4] Running benchmarks..."
echo "Test 1 (Real FS):"
cd "$SOURCE_DIR"
"$TEST_DIR/bench_fs"

# Test 2: VFS (with shim, like test_shim_basic.sh)
echo "Test 2 (VFS with shim):"
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VR_THE_SOURCE="$CAS_ROOT"
export VRIFT_MANIFEST="$SOURCE_DIR/vrift.manifest"
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_DEBUG=1

cd "$TEST_DIR"
"$TEST_DIR/bench"

echo ""
echo "✅ Benchmark complete"
echo ""
echo "=== Daemon Log Summary ==="
grep -i "exported\|mmap" "$TEST_DIR/daemon.log" 2>/dev/null || echo "(no mmap logs)"
