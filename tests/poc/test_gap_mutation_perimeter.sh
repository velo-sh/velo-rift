#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== P0 Gap Test: Mutation Perimeter (macOS) ==="

# Compile C test program
cat > "$TEST_DIR/mutation_test.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>
#include <sys/xattr.h>

int test_chmod(const char *path) {
    printf("Testing chmod on %s...\n", path);
    if (chmod(path, 0644) == 0) {
        printf("  chmod SUCCESS (bypass!)\n");
        return 1;
    } else {
        printf("  chmod BLOCKED: %s\n", strerror(errno));
        return 0;
    }
}

int test_truncate(const char *path) {
    printf("Testing truncate on %s...\n", path);
    if (truncate(path, 0) == 0) {
        printf("  truncate SUCCESS (bypass!)\n");
        return 1;
    } else {
        printf("  truncate BLOCKED: %s\n", strerror(errno));
        return 0;
    }
}

int test_setxattr(const char *path) {
    printf("Testing setxattr on %s...\n", path);
    if (setxattr(path, "user.test", "value", 5, 0, 0) == 0) {
        printf("  setxattr SUCCESS (bypass!)\n");
        return 1;
    } else {
        printf("  setxattr BLOCKED: %s\n", strerror(errno));
        return 0;
    }
}

int test_chflags(const char *path) {
    printf("Testing chflags on %s...\n", path);
    if (chflags(path, 0x10) == 0) {  // UF_IMMUTABLE
        printf("  chflags SUCCESS (bypass!)\n");
        chflags(path, 0);  // cleanup
        return 1;
    } else {
        printf("  chflags BLOCKED: %s\n", strerror(errno));
        return 0;
    }
}

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <path>\n", argv[0]);
        return 2;
    }
    const char *path = argv[1];
    
    int bypassed = 0;
    bypassed += test_chmod(path);
    bypassed += test_truncate(path);
    bypassed += test_setxattr(path);
    bypassed += test_chflags(path);
    
    printf("\n=== Summary ===\n");
    if (bypassed == 0) {
        printf("✅ PASS: All mutations blocked\n");
        return 0;
    } else {
        printf("❌ FAIL: %d mutation(s) bypassed\n", bypassed);
        return 1;
    }
}
EOF

gcc -o "$TEST_DIR/mutation_test" "$TEST_DIR/mutation_test.c"

# Prepare VFS
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "PROTECTED_CONTENT_1234567890" > "$VELO_PROJECT_ROOT/mutation_test.txt"

# Setup Shim
if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi
export VRIFT_SOCKET_PATH="/tmp/vrift.sock"
export VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT"

"$TEST_DIR/mutation_test" "$VELO_PROJECT_ROOT/mutation_test.txt"
RET=$?

rm -rf "$TEST_DIR"
exit $RET
