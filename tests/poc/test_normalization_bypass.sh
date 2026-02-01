#!/bin/bash
# Test: Path Normalization Bypass (Traversal Exploit)
# Goal: Verify that the shim correctly handles '..' in paths to prevent VFS escape.

set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SHIM_PATH="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"

if [[ "$(uname)" != "Darwin" ]]; then
    SHIM_PATH="${PROJECT_ROOT}/target/debug/libvelo_shim.so"
fi

echo "=== Test: Path Normalization Bypass ==="

# Create a dummy file on host that we'll try to 'escape' to
echo "host_secret" > /tmp/vrift_host_secret.txt

export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_VFS_PREFIX="/vrift"

# We try to access a host file using a virtual prefix + traversal
# The shim currently uses naive starts_with("/vrift")
# If we call open("/vrift/../tmp/vrift_host_secret.txt"), 
# and the shim doesn't normalize, it will see the prefix, 
# try to query the manifest for "/vrift/../tmp/...", fail, 
# and then call real_open("/vrift/../tmp/...").
# The host OS will resolve this to "/tmp/vrift_host_secret.txt" and succeed.
# This proves a VFS boundary bypass.

cat <<EOF > /tmp/test_bypass.c
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>

int main() {
    int fd = open("/vrift/../tmp/vrift_host_secret.txt", O_RDONLY);
    if (fd >= 0) {
        printf("BYPASS SUCCESS: Opened host file via traversal!\n");
        close(fd);
        return 1;
    } else {
        printf("BYPASS FAILED: Host file protected or path rejected.\n");
        return 0;
    }
}
EOF
gcc /tmp/test_bypass.c -o /tmp/test_bypass

if /tmp/test_bypass; then
    echo ""
    echo "❌ FAIL: VFS prefix bypass detected via traversal exploit!"
    exit 1
else
    echo ""
    echo "✅ PASS: Traversal correctly rejected or failed to escape."
fi
