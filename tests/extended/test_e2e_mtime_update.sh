#!/bin/bash
# RFC-0047 Critical E2E Test: utimes/Mtime for Incremental Builds
#
# This test PROVES that utimes updates VFS mtime correctly.
# If this fails, Make/Ninja incremental builds WILL break.
#
# Scenario:
# 1. Create project, ingest
# 2. Touch a header file (utimes)
# 3. Verify stat sees updated mtime
# 4. If mtime unchanged, Make skips rebuild → WRONG!

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== E2E Test: utimes/Mtime for Incremental Builds ==="
echo ""

# Build
echo "[1] Building components..."
(cd "$PROJECT_ROOT" && cargo build -p vrift-shim -p vrift-cli 2>/dev/null)

SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
CLI_PATH="${PROJECT_ROOT}/target/debug/vrift"

# Create test project
TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/project/src"
echo "int main() { return 0; }" > "$TEST_DIR/project/src/main.c"
echo "// original header" > "$TEST_DIR/project/src/config.h"

echo "[2] Created test project: $TEST_DIR/project"

# Ingest
echo "[3] Ingesting project..."
cd "$TEST_DIR/project"
"$CLI_PATH" ingest . 2>&1 | grep -E "Complete|files" | head -1 || true
MANIFEST_PATH="$TEST_DIR/project/.vrift/manifest.lmdb"

# Create test program
TEST_PROG="$TEST_DIR/test_mtime"
cat > "${TEST_PROG}.c" << 'CEOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <project_root>\n", argv[0]);
        return 1;
    }
    
    const char *root = argv[1];
    char path[1024];
    struct stat st_before, st_after;
    int pass = 0, fail = 0;
    
    printf("\n=== Mtime Test for Incremental Builds ===\n");
    printf("Project Root: %s\n\n", root);
    
    snprintf(path, sizeof(path), "%s/src/config.h", root);
    
    // Get mtime before
    printf("[1] stat(config.h) BEFORE touch\n");
    if (stat(path, &st_before) != 0) {
        printf("    ❌ stat failed: %s\n", strerror(errno));
        return 1;
    }
    printf("    mtime: %ld\n", (long)st_before.st_mtime);
    
    // Wait a bit to ensure mtime change is visible
    sleep(1);
    
    // Touch the file using utimes
    printf("\n[2] utimes(config.h) - simulate 'touch'\n");
    struct timeval times[2];
    gettimeofday(&times[0], NULL);  // atime = now
    gettimeofday(&times[1], NULL);  // mtime = now
    times[1].tv_sec += 100;  // Set mtime to future for clear difference
    
    if (utimes(path, times) != 0) {
        printf("    ❌ utimes failed: %s\n", strerror(errno));
        if (errno == EROFS) {
            printf("    ❌ CRITICAL: EROFS means VFS mtime is READ-ONLY!\n");
            printf("       Make/Ninja incremental builds will BREAK!\n");
        }
        fail++;
    } else {
        printf("    ✅ utimes succeeded\n");
        pass++;
    }
    
    // Get mtime after
    printf("\n[3] stat(config.h) AFTER touch\n");
    if (stat(path, &st_after) != 0) {
        printf("    ❌ stat failed: %s\n", strerror(errno));
        fail++;
    } else {
        printf("    mtime: %ld\n", (long)st_after.st_mtime);
        
        if (st_after.st_mtime > st_before.st_mtime) {
            printf("    ✅ PASS: mtime UPDATED correctly!\n");
            printf("       Make/Ninja will see the change and rebuild\n");
            pass++;
        } else {
            printf("    ❌ FAIL: mtime UNCHANGED!\n");
            printf("       Before: %ld, After: %ld\n", 
                   (long)st_before.st_mtime, (long)st_after.st_mtime);
            printf("       Make/Ninja will SKIP rebuild - WRONG!\n");
            fail++;
        }
    }
    
    // Summary
    printf("\n=== Results ===\n");
    printf("Passed: %d\n", pass);
    printf("Failed: %d\n", fail);
    
    if (fail == 0) {
        printf("\n✅ MTIME UPDATES WORK - Incremental builds OK!\n");
        return 0;
    } else {
        printf("\n❌ MTIME BROKEN - Make/Ninja will malfunction!\n");
        return 1;
    }
}
CEOF

gcc -o "$TEST_PROG" "${TEST_PROG}.c"
echo "[4] Compiled test program"

# Run with shim
echo ""
echo "[5] Running test with shim injection..."

export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_MANIFEST="$MANIFEST_PATH"

cd "$TEST_DIR/project"
"$TEST_PROG" "$TEST_DIR/project"
TEST_RESULT=$?

# Cleanup
unset DYLD_INSERT_LIBRARIES
echo ""
echo "[6] Cleanup..."
rm -rf "$TEST_DIR" 2>/dev/null || true

exit $TEST_RESULT
