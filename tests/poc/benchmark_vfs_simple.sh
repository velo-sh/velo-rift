#!/bin/bash
# SIMPLIFIED VFS vs FS Benchmark - No vrift activate, just raw test

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== VFS vs FS Benchmark (Simplified) ==="
echo ""

# Reuse existing test dir if available
TEST_DIR=$(ls -d /tmp/vrift_vfs_bench_* 2>/dev/null | head -1)

if [[ -z "$TEST_DIR" || ! -d "$TEST_DIR/node_modules" ]]; then
    echo "Creating new test directory..."
    TEST_DIR="/tmp/vrift_vfs_bench_$$"
    mkdir -p "$TEST_DIR"
    cd "$TEST_DIR"
    
    # Setup
    npm init -y >/dev/null
    cp "$PROJECT_ROOT/examples/benchmarks/medium_package.json" package.json
    echo "Installing dependencies..."
    npm install >/dev/null 2>&1
    
    FILE_COUNT=$(find node_modules -type f 2>/dev/null | wc -l | tr -d ' ')
    echo "✅ Created $FILE_COUNT files"
else
    echo "✅ Using existing test dir: $TEST_DIR"
    cd "$TEST_DIR"
fi

# Rebuild benchmark program
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

int fstat_all_files(const char *path, int *count) {
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
                fstat_all_files(fullpath, count);
            }
        }
    }
    closedir(dir);
    return 0;
}

int main() {
    int count = 0;
    
    long long start = now_ns();
    fstat_all_files("node_modules", &count);
    long long end = now_ns();
    
    double ms = (end - start) / 1000000.0;
    
    printf("Files: %d, Time: %.2f ms, Avg: %.2f µs/file\n", 
           count, ms, (ms * 1000) / count);
    
    return 0;
}
EOF

cc -O2 -o bench bench.c
codesign -f -s - bench 2>/dev/null || true

echo ""
echo "=== Test 1: Real FS (no shim) ==="
./bench

echo ""
echo "=== Test 2: With Shim (NO VFS, just overhead test) ==="
SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
DYLD_INSERT_LIBRARIES="$SHIM_PATH" ./bench

echo ""
echo "Done! This shows shim overhead on real multi-file workload."
