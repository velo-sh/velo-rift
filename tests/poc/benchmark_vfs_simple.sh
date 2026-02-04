#!/bin/bash
# SIMPLIFIED VFS vs FS Benchmark - Shim overhead test
# Uses 1000 files for reasonable test time

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "❌ Shim not found. Run: cargo build --release -p vrift-shim"
    exit 1
fi

echo "=== VFS vs FS Benchmark (Simplified) ==="
echo ""

# Setup test directory with 1000 files
TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

echo "Creating 1000 test files..."
mkdir -p "$TEST_DIR/testdir"
for i in $(seq 1 1000); do
    echo "test data $i" > "$TEST_DIR/testdir/file_$i.txt"
done
echo "✅ Created 1000 files"

# Create benchmark program
cat > "$TEST_DIR/bench.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <dirent.h>
#include <string.h>
#include <time.h>

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int scan_dir(const char *path, int *count) {
    DIR *dir = opendir(path);
    if (!dir) return -1;
    
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (entry->d_name[0] == '.') continue;
        
        char fullpath[4096];
        snprintf(fullpath, sizeof(fullpath), "%s/%s", path, entry->d_name);
        
        struct stat sb;
        if (lstat(fullpath, &sb) == 0) {
            (*count)++;
            if (S_ISDIR(sb.st_mode)) {
                scan_dir(fullpath, count);
            }
        }
    }
    closedir(dir);
    return 0;
}

int main() {
    int count = 0;
    long long start = now_ns();
    scan_dir("testdir", &count);
    long long end = now_ns();
    
    double ms = (end - start) / 1000000.0;
    printf("Files: %d, Time: %.2f ms, Avg: %.2f µs/file\n", 
           count, ms, count > 0 ? (ms * 1000) / count : 0);
    return 0;
}
EOF

cd "$TEST_DIR"
cc -O2 -o bench bench.c
codesign -f -s - bench 2>/dev/null || true

echo ""
echo "=== Test 1: Real FS (no shim) ==="
./bench

echo ""
echo "=== Test 2: With Shim (overhead test) ==="
DYLD_INSERT_LIBRARIES="$SHIM_PATH" VRIFT_DEBUG=0 ./bench

echo ""
echo "✅ Done! This shows shim overhead on multi-file workload."
