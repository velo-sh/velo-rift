#!/bin/bash
set -e
PROJECT_ROOT=$(pwd)
TEST_DIR="/tmp/vrift-p8-$(date +%s)"
mkdir -p "$TEST_DIR/project/subdir"
echo "data" > "$TEST_DIR/project/file.txt"

export VRIFT_CAS_ROOT="$TEST_DIR/the_source"
export VRIFT_VFS_PREFIX="/vrift"
mkdir -p "$VRIFT_CAS_ROOT"

# 1. Ingest
echo "📦 Ingesting project..."
./target/release/vrift ingest "$TEST_DIR/project" --output "$TEST_DIR/project/vrift.manifest" --prefix ""

# 2. Start Daemon
echo "🔍 Starting Daemon..."
pkill vriftd || true
sleep 1
./target/release/vriftd start > "$TEST_DIR/vriftd.log" 2>&1 &
DAEMON_PID=$!
sleep 2

# 3. Compile Test Program
cat <<EOF > "$TEST_DIR/test_pkg.c"
#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>
#include <limits.h>
#include <string.h>

int main() {
    char cwd[PATH_MAX];
    
    if (getcwd(cwd, sizeof(cwd)) != NULL) {
        printf("[1] Initial CWD: %s\n", cwd);
    }
    
    printf("[2] Testing chdir(/vrift/subdir)...\n");
    if (chdir("/vrift/subdir") == 0) {
        printf("SUCCESS: chdir(/vrift/subdir) worked!\n");
    } else {
        perror("chdir failed");
        return 1;
    }
    
    if (getcwd(cwd, sizeof(cwd)) != NULL) {
        printf("[3] New CWD: %s\n", cwd);
        if (strcmp(cwd, "/vrift/subdir") == 0) {
            printf("SUCCESS: getcwd returns virtual path!\n");
        } else {
            printf("FAIL: getcwd returns unexpected path: %s\n", cwd);
            return 1;
        }
    }
    
    char resolved[1024];
    printf("[4] Testing realpath(/vrift/file.txt)...\n");
    if (realpath("/vrift/file.txt", resolved)) {
        printf("REALPATH: %s\n", resolved);
        if (strcmp(resolved, "/vrift/file.txt") == 0) {
            printf("SUCCESS: realpath resolves to virtual path!\n");
        } else {
            printf("FAIL: realpath leaked: %s\n", resolved);
            return 1;
        }
    } else {
        perror("realpath failed");
        return 1;
    }
    
    printf("[5] Testing mutation safety...\n");
    if (unlink("/vrift/file.txt") != 0) {
        printf("SUCCESS: unlink(/vrift/file.txt) correctly blocked (EROFS)!\n");
    } else {
        printf("FAIL: unlink succeeded? Security violation!\n");
        return 1;
    }
    
    return 0;
}
EOF
gcc "$TEST_DIR/test_pkg.c" -o "$TEST_DIR/test_pkg"
codesign --force --sign - "$TEST_DIR/test_pkg"

# 4. Run Test
echo "🚀 Executing compatibility test..."
export DYLD_INSERT_LIBRARIES="$PROJECT_ROOT/target/debug/libvrift_shim.dylib"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_MANIFEST="$TEST_DIR/project/vrift.manifest"
export VRIFT_DEBUG=1

"$TEST_DIR/test_pkg"

echo "✅ Phase 8 Compatibility Verified!"
kill $DAEMON_PID
