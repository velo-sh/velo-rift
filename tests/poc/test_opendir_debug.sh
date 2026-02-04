#!/bin/bash
# test_opendir_debug.sh - Run opendir with shim debug output
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

mkdir -p "$TEST_DIR/testdir"
echo "test" > "$TEST_DIR/testdir/file.txt"

cat > "$TEST_DIR/test.c" << 'CEOF'
#include <stdio.h>
#include <dirent.h>

int main(void) {
    fprintf(stderr, "[C] Before opendir\n");
    fflush(stderr);
    DIR *d = opendir("testdir");
    fprintf(stderr, "[C] After opendir: %p\n", d);
    fflush(stderr);
    if (d) {
        closedir(d);
        printf("OK\n");
    } else {
        printf("FAIL\n");
    }
    return 0;
}
CEOF

cd "$TEST_DIR"
cc -o test test.c
codesign -s - test 2>/dev/null || true

echo "=== Running with shim + debug ==="
DYLD_INSERT_LIBRARIES="$SHIM" VRIFT_DEBUG=1 ./test 2>&1 &
PID=$!

for i in 1 2 3 4; do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        wait $PID
        echo "✅ PASS"
        exit 0
    fi
    echo "Waiting... ($i)"
done

echo "❌ HANG after 2s"
kill -9 $PID 2>/dev/null
exit 1
