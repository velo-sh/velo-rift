#!/bin/bash
# test_opendir_hang.sh - Test if opendir hangs without VFS setup
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

mkdir -p "$TEST_DIR/testdir"
echo "test" > "$TEST_DIR/testdir/file.txt"

cat > "$TEST_DIR/test.c" << 'CEOF'
#include <stdio.h>
#include <dirent.h>

int main(void) {
    DIR *d = opendir("testdir");
    if (d) {
        struct dirent *e;
        while ((e = readdir(d)) != NULL) {
            printf("Entry: %s\n", e->d_name);
        }
        closedir(d);
    } else {
        printf("opendir failed\n");
    }
    printf("Done\n");
    return 0;
}
CEOF

cd "$TEST_DIR"
cc -o test test.c
codesign -s - test 2>/dev/null || true

echo "=== Without shim ==="
./test

echo "=== With shim (should complete in < 2s) ==="
DYLD_INSERT_LIBRARIES="$SHIM" ./test &
PID=$!

for i in 1 2 3 4; do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        wait $PID
        echo "✅ PASS"
        exit 0
    fi
done

echo "❌ HANG"
kill -9 $PID 2>/dev/null
exit 1
