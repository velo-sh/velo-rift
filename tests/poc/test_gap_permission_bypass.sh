#!/bin/bash
# No set -e to allow proper error handling
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== P0 Gap Test: Permission Bypass (CAS Mode Corruption) ==="

# Compile C test program
cat > "$TEST_DIR/perm_test.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <errno.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc != 2) {
        fprintf(stderr, "Usage: %s <path>\n", argv[0]);
        return 1;
    }
    
    printf("Testing chmod 644 on %s...\n", argv[1]);
    if (chmod(argv[1], 0644) == 0) {
        printf("chmod SUCCESS (bypass!)\n");
        return 1; // Fail - chmod should be blocked
    } else {
        printf("chmod BLOCKED: %s\n", strerror(errno));
        return 0; // Pass - chmod was blocked
    }
}
EOF

clang -o "$TEST_DIR/perm_test" "$TEST_DIR/perm_test.c"

# Prepare VFS
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "IMMUTABLE_DATA" > "$VELO_PROJECT_ROOT/protected.txt"
chmod 444 "$VELO_PROJECT_ROOT/protected.txt"

# Setup Shim
if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi
export VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT"

# Run test with shim
RESULT=$("$TEST_DIR/perm_test" "$VELO_PROJECT_ROOT/protected.txt" 2>&1)
EXIT_CODE=$?
echo "$RESULT"

rm -rf "$TEST_DIR"

if [[ $EXIT_CODE -eq 0 ]]; then
    echo "✅ PASS: chmod blocked or virtualized."
    exit 0
else
    echo "❌ FAIL: Permission Bypass. chmod succeeded on virtual path."
    exit 1
fi
