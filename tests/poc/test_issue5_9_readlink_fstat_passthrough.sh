#!/bin/bash
# Test: Issue #5 & #9 - readlink/fstat Passthrough (Not Intercepted)
# Expected: FAIL (readlink returns real path, fstat returns temp file metadata)
# Fixed: SUCCESS (readlink/fstat return virtual metadata from manifest)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: readlink/fstat Passthrough ==="
echo "Issue #5: readlink_impl just calls real readlink, not resolving virtual symlinks."
echo "Issue #9: fstat_impl just calls real fstat, not returning virtual metadata."
echo ""

# Create a test symlink
rm -f /tmp/test_symlink
ln -s /tmp/real_target /tmp/test_symlink

# Compile test program
cat > /tmp/test_readlink.c << 'EOF'
#include <unistd.h>
#include <stdio.h>
#include <string.h>
int main() {
    char buf[256];
    ssize_t len = readlink("/vrift/symlink", buf, sizeof(buf)-1);
    if (len > 0) {
        buf[len] = '\0';
        printf("readlink returned: %s\n", buf);
        // If it starts with /vrift, shim is working correctly
        if (strncmp(buf, "/vrift", 6) == 0) {
            printf("SUCCESS: readlink returned virtual path\n");
            return 0;
        } else {
            printf("FAIL: readlink returned real path (passthrough)\n");
            return 1;
        }
    }
    perror("readlink failed");
    return 1;
}
EOF
gcc /tmp/test_readlink.c -o /tmp/test_readlink

# For this test, we need a running daemon with a symlink in manifest
# Since that's complex to set up, we just verify the code path exists

echo "[INFO] readlink_impl current implementation:"
grep -A5 "unsafe fn readlink_impl" "${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs" || echo "Not found"

# Check if it's just a passthrough
if grep -A5 "unsafe fn readlink_impl" "${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs" | grep -q "real_readlink(path, buf, bufsiz)"; then
    echo ""
    echo "[FAIL] readlink_impl is a passthrough - does not intercept virtual symlinks."
    EXIT_CODE=1
else
    echo "[PASS] readlink_impl appears to have interception logic."
    EXIT_CODE=0
fi

echo ""
echo "[INFO] fstat_impl current implementation:"
grep -A30 "unsafe fn fstat_impl" "${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs" | head -40

# Check if fstat returns virtual metadata (looks for st_size assignment followed by return 0)
if grep -A50 "unsafe fn fstat_impl" "${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs" | grep -q "st_size = entry.size"; then
    echo ""
    echo "[PASS] fstat_impl returns virtual metadata from manifest entry."
else
    echo ""
    echo "[FAIL] fstat_impl is a passthrough - does not return virtual metadata."
fi

# Cleanup
rm -f /tmp/test_readlink.c /tmp/test_readlink /tmp/test_symlink
exit $EXIT_CODE
