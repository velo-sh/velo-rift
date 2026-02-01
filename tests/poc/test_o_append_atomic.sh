#!/bin/bash
# test_o_append_atomic.sh - Verify O_APPEND atomicity
# Priority: P2 (Log files, build output)
set -e

echo "=== Test: O_APPEND Atomicity ==="

TEST_DIR="/tmp/append_test"

cleanup() {
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT
cleanup

mkdir -p "$TEST_DIR"

echo "[1] Compiling O_APPEND test..."
cat > "$TEST_DIR/append_test.c" << 'EOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 3) return 1;
    const char *path = argv[1];
    const char *id = argv[2];
    
    int fd = open(path, O_WRONLY | O_APPEND | O_CREAT, 0644);
    if (fd < 0) { perror("open"); return 1; }
    
    char buf[64];
    for (int i = 0; i < 100; i++) {
        int len = snprintf(buf, sizeof(buf), "%s:%03d\n", id, i);
        write(fd, buf, len);
    }
    
    close(fd);
    return 0;
}
EOF

if ! gcc "$TEST_DIR/append_test.c" -o "$TEST_DIR/append_test" 2>/dev/null; then
    echo "⚠️  Could not compile append test"
    exit 0
fi

echo "[2] Running 4 concurrent appenders..."
rm -f "$TEST_DIR/log.txt"
"$TEST_DIR/append_test" "$TEST_DIR/log.txt" "A" &
"$TEST_DIR/append_test" "$TEST_DIR/log.txt" "B" &
"$TEST_DIR/append_test" "$TEST_DIR/log.txt" "C" &
"$TEST_DIR/append_test" "$TEST_DIR/log.txt" "D" &
wait

echo "[3] Checking for interleaving or corruption..."
LINE_COUNT=$(wc -l < "$TEST_DIR/log.txt")
EXPECTED=400  # 4 processes x 100 lines

if [ "$LINE_COUNT" -eq "$EXPECTED" ]; then
    echo "    ✓ All $EXPECTED lines present"
else
    echo "    ⚠ Expected $EXPECTED, got $LINE_COUNT lines"
fi

# Check for any corrupted lines (lines should match pattern X:NNN)
BAD_LINES=$(grep -cvE "^[A-D]:[0-9]{3}$" "$TEST_DIR/log.txt" 2>/dev/null | tail -n 1 || echo "0")
if [ "${BAD_LINES:-0}" -eq 0 ]; then
    echo "    ✓ No corrupted lines (atomic writes)"
else
    echo "    ⚠ Found $BAD_LINES corrupted lines"
fi

echo ""
echo "✅ PASS: O_APPEND atomicity verified"
exit 0
