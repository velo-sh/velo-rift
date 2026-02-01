#!/bin/bash
# test_fail_unlink_cas.sh - Proof of Failure: unlink() hitting read-only CAS
# Priority: P0 (Blocker)
set -e

echo "=== Proof of Failure: unlink() Hitting CAS ==="

TEST_DIR="/tmp/unlink_fail"
CAS_DIR="$TEST_DIR/cas_store"
mkdir -p "$CAS_DIR"
echo "cas content" > "$CAS_DIR/blob1"
chmod 444 "$CAS_DIR/blob1" # Make it read-only like a real CAS

export VRIFT_VFS_PREFIX="/vrift/test"
# In a real scenario, /vrift/test/file would map to $CAS_DIR/blob1

echo "[1] Attempting unlink in virtual path (hitting physical CAS)..."
cat > "$TEST_DIR/unlink_test.c" << 'EOF'
#include <stdio.h>
#include <unistd.h>

int main() {
    if (unlink("/vrift/test/file") == 0) {
        printf("UNLINK_SUCCESS\n");
        return 0;
    } else {
        perror("UNLINK_FAILED");
        return 1;
    }
}
EOF

gcc "$TEST_DIR/unlink_test.c" -o "$TEST_DIR/unlink_test"

export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvelo_shim.dylib

# This will fail with ENOENT because /vrift/test/file doesn't exist on host,
# OR if the app somehow resolve it to the CAS path, it would fail with EACCES/EPERM.
# Either way, shim passthrough is the problem.
OUTPUT=$("$TEST_DIR/unlink_test" 2>&1)
echo "$OUTPUT"

if echo "$OUTPUT" | grep -q "No such file or directory"; then
    echo "    ❌ PROVED: unlink() bypassed shim and hit host OS (ENOENT)"
else
    echo "    ❓ UNEXPECTED RESULT"
fi

echo ""
echo "Conclusion: unlink() MUST be intercepted to prevent host leakage/failure."
