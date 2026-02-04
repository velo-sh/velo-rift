#!/bin/bash
# test_1000_lstat_only.sh - Test 1000-file lstat (no opendir) with shim
# To isolate if hang is in stat vs dir operations

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

echo "Creating 1000 files..."
for i in $(seq 1 1000); do echo "test" > "$TEST_DIR/file_$i.txt"; done

cat > "$TEST_DIR/bench.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
int main() {
    struct stat sb;
    for (int i = 1; i <= 1000; i++) {
        char path[256];
        snprintf(path, sizeof(path), "file_%d.txt", i);
        if (lstat(path, &sb) != 0) {
            printf("lstat failed at %d\n", i);
            return 1;
        }
    }
    printf("Done 1000 lstat only\n");
    return 0;
}
EOF

cd "$TEST_DIR"
cc -O2 -o bench bench.c
codesign -s - bench 2>/dev/null || true

echo "=== Without shim ==="
time ./bench

echo "=== With shim (should complete in < 5s) ==="
DYLD_INSERT_LIBRARIES="$SHIM" ./bench &
PID=$!

for i in $(seq 1 10); do
    sleep 0.5
    if ! kill -0 $PID 2>/dev/null; then
        wait $PID
        echo "✅ PASS (completed in < ${i}*0.5s)"
        exit 0
    fi
done

echo "❌ HANG after 5s"
kill -9 $PID 2>/dev/null
exit 1
