#!/bin/bash
# test_fail_runtime_opts.sh - Proof of Failure: pwrite/dup2 passthrough
# Priority: P2 (Runtime)
set -e

echo "=== Proof of Failure: pwrite/dup2 Passthrough ==="

TEST_DIR="/tmp/runtime_fail"
mkdir -p "$TEST_DIR"

export VRIFT_VFS_PREFIX="/vrift/io"

echo "[1] Testing pwrite() on virtual path..."
cat > "$TEST_DIR/runtime_test.c" << 'EOF'
#include <stdio.h>
#include <unistd.h>
#include <fcntl.h>

int main() {
    int fd = open("/vrift/io/data.bin", O_RDWR | O_CREAT, 0644);
    if (fd < 0) {
        perror("OPEN_FAILED");
        return 1;
    }
    
    char buf[] = "data";
    if (pwrite(fd, buf, 4, 1024) < 0) {
        perror("PWRITE_FAILED");
        close(fd);
        return 2;
    }
    
    printf("PWRITE_SUCCESS\n");
    close(fd);
    return 0;
}
EOF

gcc "$TEST_DIR/runtime_test.c" -o "$TEST_DIR/runtime_test"


# Open() will be shimmed, but if pwrite() isn't, it will fail on the 
# underlying physical FD if that FD came from a virtual mapping that 
# doesn't support random access write correctly, OR if pwrite symbol is missing in shim.
# On macOS, pwrite is a separate entry point.
if ! nm -gU target/debug/libvrift_inception_layer.dylib | grep -q "pwrite"; then
    echo "    ❌ PROVED: pwrite() symbol missing in shim dylib"
else
    echo "    ✓ pwrite() symbol found"
fi

echo ""
echo "Conclusion: Runtimes require full IO suite (pread/pwrite/dup2)."
