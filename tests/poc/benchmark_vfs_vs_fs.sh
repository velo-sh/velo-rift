#!/bin/bash
# VFS vs FS Performance Benchmark
# Demonstrates VFS stat caching performance vs real disk I/O

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Find binaries
if [ -f "${PROJECT_ROOT}/target/release/vrift" ]; then
    VRIFT_BIN="${PROJECT_ROOT}/target/release/vrift"
    VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"
    SHIM_PATH="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
    VDIRD_BIN="${PROJECT_ROOT}/target/release/vrift-vdird"
else
    echo "❌ ERROR: Release binaries not found. Run: cargo build --release"
    exit 1
fi

# Ensure vdir_d symlink for vDird subprocess model
[ -f "$VDIRD_BIN" ] && [ ! -e "$(dirname "$VRIFTD_BIN")/vdir_d" ] && \
    ln -sf "vrift-vdird" "$(dirname "$VRIFTD_BIN")/vdir_d"

echo "=== VFS vs FS Benchmark (Stat Caching Test) ==="
echo ""

# Setup test workspace
TEST_DIR="/tmp/vrift_vfs_bench_$$"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

PHYSICAL_ROOT="$TEST_DIR/source"
CAS_ROOT="$TEST_DIR/cas"
mkdir -p "$PHYSICAL_ROOT" "$CAS_ROOT"

# 1. Create project with npm install
echo "📦 Step 1: Creating project with npm install..."
npm init -y > /dev/null
cp "$PROJECT_ROOT/examples/benchmarks/medium_package.json" package.json 2>/dev/null || {
    echo "{\"dependencies\": {\"express\": \"^4.18.0\"}}" > package.json
}
npm install > /dev/null 2>&1

# Move node_modules into physical root
mv node_modules "$PHYSICAL_ROOT/"

FILE_COUNT=$(find "$PHYSICAL_ROOT/node_modules" -type f 2>/dev/null | wc -l | tr -d ' ')
echo "✅ Created $FILE_COUNT files in physical root"

# Create C benchmark program
cat > bench.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <dirent.h>
#include <string.h>
#include <time.h>

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int lstat_all_files(const char *path, int *count) {
    struct dirent *entry;
    DIR *dir = opendir(path);
    if (!dir) return -1;

    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0)
            continue;

        char fullpath[4096];
        snprintf(fullpath, sizeof(fullpath), "%s/%s", path, entry->d_name);

        struct stat sb;
        if (lstat(fullpath, &sb) == 0) {
            (*count)++;
            if (S_ISDIR(sb.st_mode)) {
                lstat_all_files(fullpath, count);
            }
        }
    }
    closedir(dir);
    return 0;
}

int main() {
    int count = 0;
    long long start = now_ns();
    lstat_all_files("node_modules", &count);
    long long end = now_ns();
    
    double ms = (end - start) / 1000000.0;
    printf("Files: %d, Time: %.2f ms, Avg: %.2f µs/file\n", 
           count, ms, (ms * 1000) / count);
    
    return 0;
}
EOF

cc -O2 -o bench bench.c
codesign -f -s - bench 2>/dev/null || true

# 2. Ingest into VFS
echo ""
echo "🚀 Step 2: Ingesting to VFS (vrift ingest)..."
export VR_THE_SOURCE="$(realpath "$CAS_ROOT")"
# Store manifest in PHYSICAL_ROOT so shim mmap path aligns
mkdir -p "$PHYSICAL_ROOT/.vrift"
"$VRIFT_BIN" ingest "$PHYSICAL_ROOT" --output "$PHYSICAL_ROOT/.vrift/manifest.lmdb"

# 3. Start vriftd
echo "🔧 Step 3: Starting vriftd..."
pkill -9 vriftd 2>/dev/null || true
sleep 0.5

# NOTE: VRIFT_MANIFEST determines where shim looks for mmap
export VRIFT_MANIFEST="$PHYSICAL_ROOT/.vrift/manifest.lmdb"
export VRIFT_VFS_PREFIX="$PHYSICAL_ROOT"
"$VRIFTD_BIN" > "$TEST_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!

cleanup() {
    kill $DAEMON_PID 2>/dev/null || true
    # Clear uchg flag from CAS files before removal
    chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Wait for daemon
MAX_RETRIES=10
RETRY_COUNT=0
while [ ! -S "/tmp/vrift.sock" ] && [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
    sleep 0.5
    RETRY_COUNT=$((RETRY_COUNT + 1))
done

if [ ! -S "/tmp/vrift.sock" ]; then
    echo "❌ ERROR: vriftd failed to start"
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

echo "✅ VFS ready, manifest loaded"

# 4. Test 1: Real FS (no shim)
echo ""
echo "=== Test 1: Real FS (no shim, direct disk I/O) ==="
cd "$PHYSICAL_ROOT"
../bench

# 5. Test 2: VFS (with shim, mmap cache)
echo ""
echo "=== Test 2: VFS (with shim, mmap stat cache) ==="

# Create warmup binary that traverses ALL files to ensure mmap is fully populated
cat > warmup.c << 'EOF'
#include <sys/stat.h>
#include <stdio.h>
#include <dirent.h>
#include <string.h>

void warmup_all(const char *path, int *count) {
    struct dirent *entry;
    DIR *dir = opendir(path);
    if (!dir) return;
    
    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0)
            continue;
        
        char fullpath[4096];
        snprintf(fullpath, sizeof(fullpath), "%s/%s", path, entry->d_name);
        
        struct stat sb;
        if (lstat(fullpath, &sb) == 0) {
            (*count)++;
            if (S_ISDIR(sb.st_mode)) {
                warmup_all(fullpath, count);
            }
        }
    }
    closedir(dir);
}

int main() {
    int count = 0;
    warmup_all("node_modules", &count);
    printf("Warmup: stat'd %d entries, mmap populated\n", count);
    return 0;
}
EOF
cc -O2 -o warmup warmup.c
codesign -f -s - warmup 2>/dev/null || true

export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
# Suppress debug output for cleaner benchmark results
export VRIFT_DEBUG=0

# Warmup: trigger workspace registration and populate mmap cache
echo "[Warmup: traversing all files to populate mmap...]" 
./warmup
sleep 2  # Give daemon time to fully export mmap

# Now run the actual benchmark with warm cache
../bench

unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE

echo ""
echo "=== Benchmark Complete ==="
echo "Expected: Test 2 should be significantly faster (memory vs disk)"
