#!/bin/bash
# Test: Issue #1 - macOS Interposition Recursion Deadlock
# Expected: FAIL (process hangs or times out)
# Fixed: SUCCESS (stat returns within 1 second)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TIMEOUT=3

echo "=== Test: macOS Interposition Recursion Deadlock ==="
echo "Issue: Shim's stat_shim calls 'stat' symbol which is intercepted, causing infinite recursion."
echo ""

# Compile a minimal test program
cat > /tmp/test_stat.c << 'EOF'
#include <sys/stat.h>
#include <stdio.h>
int main() {
    struct stat st;
    if (stat("/tmp", &st) == 0) {
        printf("SUCCESS\n");
        return 0;
    }
    perror("stat");
    return 1;
}
EOF
gcc /tmp/test_stat.c -o /tmp/test_stat

# Run with shim
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"
export VRIFT_VFS_PREFIX="/vrift"

echo "[RUN] Executing stat with shim (timeout ${TIMEOUT}s)..."

# Cross-platform timeout using background process
/tmp/test_stat > /tmp/test_stat.log 2>&1 &
STAT_PID=$!
sleep ${TIMEOUT}
if kill -0 $STAT_PID 2>/dev/null; then
    echo "[FAIL] Process TIMED OUT - recursion deadlock detected!"
    kill -9 $STAT_PID 2>/dev/null
    EXIT_CODE=1
else
    wait $STAT_PID
    STAT_EXIT=$?
    if [ $STAT_EXIT -eq 0 ]; then
        echo "[PASS] stat completed without hanging."
        cat /tmp/test_stat.log
        EXIT_CODE=0
    else
        echo "[FAIL] stat failed with error (exit code: $STAT_EXIT)."
        cat /tmp/test_stat.log
        EXIT_CODE=1
    fi
fi

# Cleanup
rm -f /tmp/test_stat.c /tmp/test_stat /tmp/test_stat.log
exit $EXIT_CODE
