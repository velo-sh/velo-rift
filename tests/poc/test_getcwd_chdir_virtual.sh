#!/bin/bash
# Test: getcwd/chdir Virtual Directory Navigation - Runtime Verification
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== Test: getcwd/chdir Virtual Directory Navigation (Runtime) ==="

# Compile test program
cat > "$TEST_DIR/chdir_test.c" << 'EOF'
#include <stdio.h>
#include <unistd.h>
#include <string.h>
int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    
    // Test chdir
    if (chdir(argv[1]) != 0) { perror("chdir"); return 1; }
    printf("chdir to %s: OK\n", argv[1]);
    
    // Test getcwd
    char cwd[1024];
    if (getcwd(cwd, sizeof(cwd)) == NULL) { perror("getcwd"); return 1; }
    printf("getcwd: %s\n", cwd);
    
    // Verify we're in the right directory
    if (strstr(cwd, "src") != NULL) {
        printf("✅ PASS: getcwd/chdir work correctly\n");
        return 0;
    } else {
        printf("❌ FAIL: getcwd returned unexpected path\n");
        return 1;
    }
}
EOF
gcc -o "$TEST_DIR/chdir_test" "$TEST_DIR/chdir_test.c"

# Prepare VFS workspace
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
mkdir -p "$VELO_PROJECT_ROOT/src"
echo "test" > "$VELO_PROJECT_ROOT/src/main.rs"

# Setup Shim and run test
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH="/tmp/vrift.sock" \
VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT" \
"$TEST_DIR/chdir_test" "$VELO_PROJECT_ROOT/src"
RET=$?

rm -rf "$TEST_DIR"
exit $RET
