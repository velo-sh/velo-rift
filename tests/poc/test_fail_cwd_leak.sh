#!/bin/bash
# test_fail_cwd_leak.sh - Proof of Failure: getcwd() inconsistency
# Priority: P1
set -e

echo "=== Proof of Failure: getcwd() Inconsistency ==="

TEST_DIR="/tmp/cwd_fail"
mkdir -p "$TEST_DIR/real_project"

export VRIFT_VFS_PREFIX="/vrift/project"
# Assume /vrift/project is mapped to $TEST_DIR/real_project

echo "[1] Testing getcwd() after chdir() to virtual path..."
cat > "$TEST_DIR/cwd_test.c" << 'EOF'
#include <stdio.h>
#include <unistd.h>
#include <string.h>

int main() {
    if (chdir("/vrift/project") != 0) {
        perror("chdir");
        return 1;
    }
    
    char buf[1024];
    if (getcwd(buf, sizeof(buf)) == NULL) {
        perror("getcwd");
        return 1;
    }
    
    printf("CWD: %s\n", buf);
    if (strstr(buf, "/vrift/project") != NULL) {
        printf("MATCH_VIRTUAL\n");
    } else {
        printf("MATCH_PHYSICAL\n");
    }
    return 0;
}
EOF

gcc "$TEST_DIR/cwd_test.c" -o "$TEST_DIR/cwd_test"

export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvelo_shim.dylib

# chdir() might work if shim intercepts it (it does in some versions), 
# but if getcwd() isn't intercepted, it returns the host path.
OUTPUT=$("$TEST_DIR/cwd_test" 2>&1) || EXIT_VAL=$?
echo "$OUTPUT"

if echo "$OUTPUT" | grep -q "MATCH_PHYSICAL" || echo "$OUTPUT" | grep -q "No such file or directory"; then
    echo "    ❌ PROVED: getcwd()/chdir() leaked physical path or failed (ENOENT)"
else
    echo "    ✓ paths handled correctly"
fi

echo ""
echo "Conclusion: getcwd() must be shimmed for path parity."
