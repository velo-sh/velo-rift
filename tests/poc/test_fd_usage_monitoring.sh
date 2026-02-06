#!/bin/bash
# tests/poc/test_fd_usage_monitoring.sh
set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
MANIFEST="$PROJECT_ROOT/tests/poc/vrift.manifest"

echo "=== Testing Lock-Free FD Usage Monitoring (70%/85% UX Rules) ==="

# 1. Compile helper
cat << 'EOF' > /tmp/stress_fds.c
#include <sys/resource.h>
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>

int main(int argc, char** argv) {
    if (argc < 2) return 1;
    const char* path = argv[1];

    // 0. Trigger Shim Init
    int trigger_fd = open("/dev/null", O_RDONLY);
    if (trigger_fd >= 0) close(trigger_fd);

    struct rlimit rl;
    getrlimit(RLIMIT_NOFILE, &rl);
    rl.rlim_cur = 1000;
    setrlimit(RLIMIT_NOFILE, &rl);
    printf("Soft Limit set to: 1000\n");

    int fds[1001];
    
    // 1. Trigger 70% Warning (700 files)
    printf("Opening 750 files (threshold: 700)...\n");
    for (int i = 0; i < 750; i++) {
        fds[i] = open(path, O_RDONLY);
    }
    
    // 2. Trigger 85% Critical (850 files)
    printf("Opening 120 more files (threshold: 850)...\n");
    for (int i = 750; i < 870; i++) {
        fds[i] = open(path, O_RDONLY);
    }

    // 3. Cleanup
    for (int i = 0; i < 870; i++) {
        close(fds[i]);
    }

    return 0;
}
EOF
cc /tmp/stress_fds.c -o /tmp/stress_fds

# 2. Run stress test
echo "Running stress test..."
TARGET_FILE="$PROJECT_ROOT/tests/poc/diag_test.c"
OUTPUT=$( (export VRIFT_MANIFEST="$MANIFEST"; DYLD_INSERT_LIBRARIES="$SHIM_LIB" /tmp/stress_fds "$TARGET_FILE" 2>&1) )

echo "--- Output ---"
echo "$OUTPUT"
echo "--------------"

# Verify 70% (WARNING) and 85% (CRITICAL)
if echo "$OUTPUT" | grep -q "WARNING: FD usage at 7"; then
    echo "✅ SUCCESS: 70% Warning found"
else
    echo "❌ FAILURE: 70% Warning missing"
fi

if echo "$OUTPUT" | grep -q "CRITICAL: FD usage at 8"; then
    echo "✅ SUCCESS: 85% Critical found"
else
    echo "❌ FAILURE: 85% Critical missing"
fi

# Cleanup
rm -f /tmp/stress_fds.c /tmp/stress_fds
