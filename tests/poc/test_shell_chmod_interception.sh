#!/bin/bash
# Test: Shell's chmod command interception
# Goal: Verify chmod is intercepted when run from shell
# Uses local binary copy to bypass macOS SIP

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== Test: Shell chmod Interception ==="

# OS Detection
if [ "$(uname -s)" == "Darwin" ]; then
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
    STAT_MODE_FLAG="-f %Lp"
else
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.so"
    PRELOAD_VAR="LD_PRELOAD"
    STAT_MODE_FLAG="-c %a"
fi

# Prepare workspace
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "PROTECTED" > "$VELO_PROJECT_ROOT/test.txt"
chmod 444 "$VELO_PROJECT_ROOT/test.txt"
ORIGINAL_MODE=$(stat $STAT_MODE_FLAG "$VELO_PROJECT_ROOT/test.txt")
echo "Original mode: $ORIGINAL_MODE"

# Avoid SIP and Signature issues by compiling a tiny chmod
cat <<EOF > "$TEST_DIR/tiny_chmod.c"
#include <sys/stat.h>
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char** argv) {
    if (argc < 3) return 1;
    int mode = (int)strtol(argv[1], NULL, 8);
    if (chmod(argv[2], mode) < 0) {
        perror("chmod");
        return 1;
    }
    return 0;
}
EOF
mkdir -p "$TEST_DIR/bin"
gcc "$TEST_DIR/tiny_chmod.c" -o "$TEST_DIR/bin/chmod"
CHMOD_CMD="$TEST_DIR/bin/chmod"

# Setup Shim
export "$PRELOAD_VAR"="$SHIM_LIB"
if [ "$(uname -s)" == "Darwin" ]; then
    export DYLD_FORCE_FLAT_NAMESPACE=1
fi
export VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT"

# Test: Run chmod
echo "Running: $CHMOD_CMD 644 $VELO_PROJECT_ROOT/test.txt"

if "$CHMOD_CMD" 644 "$VELO_PROJECT_ROOT/test.txt" 2>/dev/null; then
    NEW_MODE=$(stat $STAT_MODE_FLAG "$VELO_PROJECT_ROOT/test.txt")
    echo "chmod succeeded. New mode: $NEW_MODE"
    if [[ "$NEW_MODE" != "$ORIGINAL_MODE" ]]; then
        echo "❌ FAIL: chmod changed file mode (not intercepted)"
        unset "$PRELOAD_VAR" DYLD_FORCE_FLAT_NAMESPACE
        rm -rf "$TEST_DIR"
        exit 1
    else
        echo "✅ PASS: chmod succeeded but mode unchanged (virtualized)"
    fi
else
    echo "chmod returned error (blocked by shim)"
    echo "✅ PASS: Shell chmod properly blocked"
fi

unset "$PRELOAD_VAR" DYLD_FORCE_FLAT_NAMESPACE
rm -rf "$TEST_DIR"
exit 0
