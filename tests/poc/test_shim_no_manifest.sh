#!/bin/bash
# test_shim_no_manifest.sh - Test shim behavior when manifest is missing
# This should NOT hang - shim should fallback gracefully

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

if [[ ! -f "$SHIM_PATH" ]]; then
    echo "ERROR: Shim not found at $SHIM_PATH"
    exit 1
fi

# Create test binary
TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

echo "test content" > "$TEST_DIR/testfile.txt"

cat > "$TEST_DIR/stat_test.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
int main() {
    struct stat sb;
    if (lstat("testfile.txt", &sb) == 0) {
        printf("OK size=%lld\n", (long long)sb.st_size);
        return 0;
    } else {
        printf("FAIL\n");
        return 1;
    }
}
EOF

cc -O2 -o "$TEST_DIR/stat_test" "$TEST_DIR/stat_test.c"
codesign -s - "$TEST_DIR/stat_test" 2>/dev/null || true

cd "$TEST_DIR"

echo "=== Test 1: Without shim ==="
./stat_test
echo ""

echo "=== Test 2: With shim (no manifest - should fallback, NOT hang) ==="
unset VRIFT_MANIFEST VRIFT_VFS_PREFIX VRIFT_PROJECT_ROOT

# Run with background and timeout check
DYLD_INSERT_LIBRARIES="$SHIM_PATH" VRIFT_DEBUG=1 ./stat_test 2>&1 &
PID=$!

# Check every 0.5s up to 3s
for i in 1 2 3 4 5 6; do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        wait $PID
        echo ""
        echo "✅ PASS: Process completed in < ${i}*0.5s"
        exit 0
    fi
done

echo ""
echo "❌ FAIL: Process hung (still running after 3s)"
kill -9 $PID 2>/dev/null
exit 1
