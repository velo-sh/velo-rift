#!/bin/bash
# test_100_files.sh - Test 100-file lstat with shim
# Should NOT hang with empty vfs_prefix fix

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

echo "Creating 100 files..."
for i in $(seq 1 100); do echo "test" > "$TEST_DIR/file_$i.txt"; done

cat > "$TEST_DIR/bench.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
int main() {
    struct stat sb;
    for (int i = 1; i <= 100; i++) {
        char path[256];
        snprintf(path, sizeof(path), "file_%d.txt", i);
        lstat(path, &sb);
    }
    printf("Done 100\n");
    return 0;
}
EOF

cd "$TEST_DIR"
cc -O2 -o bench bench.c
codesign -s - bench 2>/dev/null || true

echo "=== Without shim ==="
./bench

echo "=== With shim (should complete in < 3s) ==="
DYLD_INSERT_LIBRARIES="$SHIM" ./bench &
PID=$!

for i in 1 2 3 4 5 6; do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        wait $PID
        echo "✅ PASS"
        exit 0
    fi
done

echo "❌ HANG after 3s"
kill -9 $PID 2>/dev/null
exit 1
