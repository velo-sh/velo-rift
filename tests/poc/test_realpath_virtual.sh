#!/bin/bash
# Test: realpath Virtual Path Resolution - Runtime Verification
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== Test: realpath Virtual Path Resolution (Runtime) ==="

# Compile test program
cat > "$TEST_DIR/realpath_test.c" << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    char *resolved = realpath(argv[1], NULL);
    if (!resolved) { perror("realpath"); return 1; }
    printf("Input:    %s\n", argv[1]);
    printf("Resolved: %s\n", resolved);
    if (strlen(resolved) > 0) {
        printf("✅ PASS: realpath resolved path\n");
        free(resolved);
        return 0;
    } else {
        printf("❌ FAIL: realpath returned empty\n");
        free(resolved);
        return 1;
    }
}
EOF
gcc -o "$TEST_DIR/realpath_test" "$TEST_DIR/realpath_test.c"

# Prepare VFS workspace
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
mkdir -p "$VELO_PROJECT_ROOT/src"
echo "test" > "$VELO_PROJECT_ROOT/src/main.rs"

# Setup Shim and run test with relative path
cd "$VELO_PROJECT_ROOT"
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH="/tmp/vrift.sock" \
VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT" \
"$TEST_DIR/realpath_test" "./src/../src/main.rs"
RET=$?

rm -rf "$TEST_DIR"
exit $RET
