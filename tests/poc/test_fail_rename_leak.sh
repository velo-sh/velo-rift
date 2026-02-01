#!/bin/bash
# test_fail_rename_leak.sh - Proof of Failure: rename() passthrough break
# Priority: P0 (Blocker)
set -e

echo "=== Proof of Failure: rename() Passthrough ==="

TEST_DIR="/tmp/rename_fail"
mkdir -p "$TEST_DIR/real_src"
echo "content" > "$TEST_DIR/real_src/a.tmp"

# Mock VFS: /vrift/path -> $TEST_DIR/real_src
export VRIFT_VFS_PREFIX="/vrift/test"
# In a real scenario, this would be in the manifest. 
# Here we just show that shim DOES NOT intercept rename.

echo "[1] Attempting rename in virtual path (expected to fail if passthrough)..."
cat > "$TEST_DIR/rename_test.c" << 'EOF'
#include <stdio.h>
#include <stdio.h>

int main() {
    if (rename("/vrift/test/a.tmp", "/vrift/test/a.o") == 0) {
        printf("RENAME_SUCCESS\n");
        return 0;
    } else {
        perror("RENAME_FAILED");
        return 1;
    }
}
EOF

gcc "$TEST_DIR/rename_test.c" -o "$TEST_DIR/rename_test"

export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvelo_shim.dylib

# This WILL fail because "/vrift/test" does not exist on the host filesystem 
# and the shim is not intercepting rename to redirect it.
if "$TEST_DIR/rename_test" 2>&1 | grep -q "No such file or directory"; then
    echo "    ❌ PROVED: rename() bypassed shim and hit host OS (ENOENT)"
else
    echo "    ✓ rename() worked (Unexpected?)"
fi || true

echo ""
echo "Conclusion: rename() MUST be intercepted to support build tools."
