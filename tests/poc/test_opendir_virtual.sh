#!/bin/bash
# Test: opendir/readdir Virtual Directory - Runtime Verification
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== Test: opendir/readdir Virtual Directory (Runtime) ==="

# Compile test program
cat > "$TEST_DIR/opendir_test.c" << 'EOF'
#include <stdio.h>
#include <dirent.h>
#include <string.h>
int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    DIR *d = opendir(argv[1]);
    if (!d) { perror("opendir"); return 1; }
    int count = 0;
    struct dirent *ent;
    while ((ent = readdir(d)) != NULL) {
        if (strcmp(ent->d_name, ".") != 0 && strcmp(ent->d_name, "..") != 0) {
            printf("  %s\n", ent->d_name);
            count++;
        }
    }
    closedir(d);
    printf("Found %d entries\n", count);
    if (count > 0) {
        printf("✅ PASS: opendir/readdir works\n");
        return 0;
    } else {
        printf("❌ FAIL: No entries found\n");
        return 1;
    }
}
EOF
gcc -o "$TEST_DIR/opendir_test" "$TEST_DIR/opendir_test.c"

# Prepare VFS workspace with files
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
mkdir -p "$VELO_PROJECT_ROOT/src"
echo "fn main() {}" > "$VELO_PROJECT_ROOT/src/main.rs"
echo "mod lib;" > "$VELO_PROJECT_ROOT/src/lib.rs"

# Setup Shim and run test
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH="/tmp/vrift.sock" \
VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT" \
"$TEST_DIR/opendir_test" "$VELO_PROJECT_ROOT/src"
RET=$?

rm -rf "$TEST_DIR"
exit $RET
