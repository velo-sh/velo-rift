#!/bin/bash
# RFC-0047 Critical E2E Test: unlink/rename Behavior
#
# This test PROVES the actual runtime behavior of unlink/rename
# with the vrift shim. If this fails, compilers WILL break.
#
# Test: Create file in workspace, ingest, then:
# 1. Try to unlink the file
# 2. Try to rename the file
# 3. Verify if these succeed or fail with EROFS

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== E2E Test: unlink/rename Compiler Compatibility ==="
echo ""

# Build
echo "[1] Building components..."
(cd "$PROJECT_ROOT" && cargo build -p vrift-inception-layer -p vrift-cli -p vrift-daemon 2>/dev/null)

SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
CLI_PATH="${PROJECT_ROOT}/target/debug/vrift"
DAEMON_PATH="${PROJECT_ROOT}/target/debug/vriftd"

# Create test workspace
TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/project/src"
echo "int main() { return 0; }" > "$TEST_DIR/project/src/main.c"
echo "// header" > "$TEST_DIR/project/src/foo.h"
echo "void foo() {}" > "$TEST_DIR/project/src/foo.c"

echo "[2] Created test workspace: $TEST_DIR/project"

# Ingest
echo "[3] Ingesting files..."
cd "$TEST_DIR/project"
"$CLI_PATH" ingest . 2>&1 | grep -E "Complete|files" || true
MANIFEST_PATH="$TEST_DIR/project/.vrift/manifest.lmdb"
echo "    Manifest: $MANIFEST_PATH"

# Create test C program
echo "[4] Creating test program..."
TEST_PROG="$TEST_DIR/test_mutation"
cat > "${TEST_PROG}.c" << 'CEOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <sys/stat.h>
#include <fcntl.h>

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <project_root>\n", argv[0]);
        return 1;
    }
    
    const char *root = argv[1];
    char path[1024];
    char new_path[1024];
    int pass = 0, fail = 0;
    
    printf("\n=== Compiler Mutation Syscall Test ===\n");
    printf("Project Root: %s\n\n", root);
    
    // Test 1: unlink
    printf("[Test 1] unlink(src/foo.h)\n");
    snprintf(path, sizeof(path), "%s/src/foo.h", root);
    
    // First verify file exists via stat
    struct stat st;
    if (stat(path, &st) != 0) {
        printf("    Pre-condition failed: file doesn't exist\n");
        fail++;
    } else {
        int ret = unlink(path);
        if (ret == 0) {
            printf("    ✅ PASS: unlink succeeded\n");
            pass++;
            
            // Verify file is gone
            if (stat(path, &st) == 0) {
                printf("    ⚠️  WARNING: File still visible after unlink (expected for VFS)\n");
            } else if (errno == ENOENT) {
                printf("    ✅ File correctly removed\n");
            }
        } else {
            printf("    ❌ FAIL: unlink returned %d, errno=%d (%s)\n", ret, errno, strerror(errno));
            if (errno == EROFS) {
                printf("    ❌ CRITICAL: EROFS breaks compilers!\n");
                printf("       GCC pattern: unlink(old.o) before write\n");
            }
            fail++;
        }
    }
    
    // Test 2: rename (atomic replace pattern)
    printf("\n[Test 2] rename(src/main.c -> src/main_old.c)\n");
    snprintf(path, sizeof(path), "%s/src/main.c", root);
    snprintf(new_path, sizeof(new_path), "%s/src/main_old.c", root);
    
    if (stat(path, &st) != 0) {
        printf("    Pre-condition failed: src/main.c doesn't exist\n");
        fail++;
    } else {
        int ret = rename(path, new_path);
        if (ret == 0) {
            printf("    ✅ PASS: rename succeeded\n");
            pass++;
            
            // Verify rename worked
            if (stat(new_path, &st) == 0) {
                printf("    ✅ New path exists\n");
            }
            if (stat(path, &st) != 0 && errno == ENOENT) {
                printf("    ✅ Old path removed\n");
            }
        } else {
            printf("    ❌ FAIL: rename returned %d, errno=%d (%s)\n", ret, errno, strerror(errno));
            if (errno == EROFS) {
                printf("    ❌ CRITICAL: EROFS breaks compilers!\n");
                printf("       GCC pattern: rename(tmp.o, final.o) for atomic replace\n");
            }
            fail++;
        }
    }
    
    // Test 3: Create new file (O_CREAT + O_EXCL)
    printf("\n[Test 3] open(O_CREAT|O_EXCL) - create new file\n");
    snprintf(path, sizeof(path), "%s/src/output.o", root);
    
    int fd = open(path, O_CREAT | O_EXCL | O_WRONLY, 0644);
    if (fd >= 0) {
        printf("    ✅ PASS: Created new file\n");
        write(fd, "test content", 12);
        close(fd);
        pass++;
        
        // Verify it exists
        if (stat(path, &st) == 0) {
            printf("    ✅ File visible via stat (size=%lld)\n", (long long)st.st_size);
        }
    } else {
        printf("    ❌ FAIL: open(O_CREAT) failed, errno=%d (%s)\n", errno, strerror(errno));
        fail++;
    }
    
    // Test 4: Truncate existing file (O_TRUNC)
    printf("\n[Test 4] open(O_TRUNC) - truncate existing\n");
    snprintf(path, sizeof(path), "%s/src/foo.c", root);
    
    fd = open(path, O_WRONLY | O_TRUNC);
    if (fd >= 0) {
        printf("    ✅ PASS: Truncate succeeded\n");
        write(fd, "// truncated", 12);
        close(fd);
        pass++;
    } else {
        printf("    ❌ FAIL: open(O_TRUNC) failed, errno=%d (%s)\n", errno, strerror(errno));
        if (errno == EROFS) {
            printf("    ❌ CRITICAL: EROFS breaks compilers!\n");
        }
        fail++;
    }
    
    // Summary
    printf("\n=== Results ===\n");
    printf("Passed: %d\n", pass);
    printf("Failed: %d\n", fail);
    
    if (fail == 0) {
        printf("\n✅ ALL MUTATION SYSCALLS WORK - Compilers will function!\n");
        return 0;
    } else if (fail >= 2) {
        printf("\n❌ CRITICAL: Multiple mutation syscalls fail\n");
        printf("   Compilers (GCC/Clang/Rust) WILL FAIL in VFS mode!\n");
        return 1;
    } else {
        printf("\n⚠️ Some syscalls fail - may cause issues\n");
        return 1;
    }
}
CEOF

gcc -o "$TEST_PROG" "${TEST_PROG}.c"
echo "    Compiled: $TEST_PROG"

# Run with shim
echo ""
echo "[5] Running test with shim injection..."
echo ""

# Behavior-based daemon check instead of pgrep
if ! "$CLI_PATH" daemon status 2>/dev/null | grep -q "running\|Operational"; then
    echo "    Starting daemon..."
    "$DAEMON_PATH" start &
    sleep 2
fi

export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_MANIFEST="$MANIFEST_PATH"
export VRIFT_VFS_PREFIX="/vrift"

cd "$TEST_DIR/project"
"$TEST_PROG" "$TEST_DIR/project"
TEST_RESULT=$?

# Cleanup
echo ""
echo "[6] Cleanup..."
rm -rf "$TEST_DIR" 2>/dev/null || true

if [[ $TEST_RESULT -eq 0 ]]; then
    echo ""
    echo "✅ E2E TEST PASSED"
    exit 0
else
    echo ""
    echo "❌ E2E TEST FAILED - Mutation syscalls broken"
    exit 1
fi
