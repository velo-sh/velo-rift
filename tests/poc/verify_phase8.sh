#!/bin/bash
set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TEST_DIR="/tmp/vrift-p8-$(date +%s)"
mkdir -p "$TEST_DIR/project/subdir"
echo "data" > "$TEST_DIR/project/file.txt"

export VR_THE_SOURCE="$TEST_DIR/the_source"
export VRIFT_VFS_PREFIX="/vrift"
mkdir -p "$VR_THE_SOURCE"

# 1. Ingest
echo "📦 Ingesting project..."
VRIFT_BIN="./target/release/vrift"
VRIFTD_BIN="./target/release/vriftd"
if [ ! -f "$VRIFT_BIN" ]; then
    VRIFT_BIN="./target/debug/vrift"
    VRIFTD_BIN="./target/debug/vriftd"
fi
"$VRIFT_BIN" ingest "$TEST_DIR/project" --prefix ""

# 2. Start Daemon
echo "🔍 Starting Daemon..."
pkill vriftd || true
sleep 1
"$VRIFTD_BIN" start > "$TEST_DIR/vriftd.log" 2>&1 &
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
# Use release inception layer with debug fallback
INCEPTION_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
if [ ! -f "$INCEPTION_LIB" ]; then
    INCEPTION_LIB="$PROJECT_ROOT/target/debug/libvrift_inception_layer.dylib"
fi
export DYLD_INSERT_LIBRARIES="$INCEPTION_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_MANIFEST="$TEST_DIR/project/.vrift/manifest.lmdb"
export VRIFT_DEBUG=1

"$TEST_DIR/test_pkg"

echo "✅ Phase 8 Compatibility Verified!"
kill $DAEMON_PID
