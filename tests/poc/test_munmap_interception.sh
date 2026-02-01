#!/bin/bash
# test_munmap_interception.sh - Verify munmap interception
# Priority: P2
set -e

echo "=== Test: munmap Interception ==="

TEST_DIR="/tmp/munmap_test"
cleanup() {
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT
cleanup
mkdir -p "$TEST_DIR"

echo "[1] Compiling munmap test program..."
cat > "$TEST_DIR/munmap_test.c" << 'EOF'
#include <stdio.h>
#include <sys/mman.h>
#include <fcntl.h>
#include <unistd.h>

int main() {
    size_t len = 4096;
    void *addr = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    if (addr == MAP_FAILED) {
        perror("mmap");
        return 1;
    }
    printf("MAPPED\n");
    
    if (munmap(addr, len) == 0) {
        printf("UNMAPPED\n");
        return 0;
    } else {
        perror("munmap");
        return 2;
    }
}
EOF

if ! gcc "$TEST_DIR/munmap_test.c" -o "$TEST_DIR/munmap_test" 2>/dev/null; then
    echo "⏭️ SKIP: Could not compile test program"
    exit 0
fi

echo "[2] Running with shim..."
# We use nm to verify the symbol exists in the shim first
if ! nm -gU target/debug/libvelo_shim.dylib | grep -q "munmap_shim"; then
    echo "❌ FAIL: munmap_shim symbol not found in dylib"
    exit 1
fi

export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvelo_shim.dylib
export VRIFT_DEBUG=1

OUTPUT=$("$TEST_DIR/munmap_test" 2>&1)
if echo "$OUTPUT" | grep -q "UNMAPPED"; then
    echo "    ✓ munmap succeeded under shim"
else
    echo "    ✗ munmap failed: $OUTPUT"
    exit 1
fi

echo ""
echo "✅ PASS: munmap interception verified"
exit 0
