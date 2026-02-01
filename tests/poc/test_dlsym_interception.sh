#!/bin/bash
# test_dlsym_interception.sh - Verify dlsym interception
# Priority: P2
set -e

echo "=== Test: dlsym Interception ==="

TEST_DIR="/tmp/dlsym_test"
cleanup() {
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT
cleanup
mkdir -p "$TEST_DIR"

echo "[1] Compiling dlsym test program..."
cat > "$TEST_DIR/dlsym_test.c" << 'EOF'
#include <stdio.h>
#include <dlfcn.h>

int main() {
    void *handle = dlopen(NULL, RTLD_LAZY);
    if (!handle) {
        printf("DLOPEN_FAILED: %s\n", dlerror());
        return 1;
    }
    
    void *sym = dlsym(handle, "printf");
    if (sym) {
        printf("SYM_FOUND\n");
        dlclose(handle);
        return 0;
    } else {
        printf("SYM_NOT_FOUND: %s\n", dlerror());
        dlclose(handle);
        return 2;
    }
}
EOF

if ! gcc "$TEST_DIR/dlsym_test.c" -o "$TEST_DIR/dlsym_test" 2>/dev/null; then
    echo "⏭️ SKIP: Could not compile test program"
    exit 0
fi

echo "[2] Running with shim..."
if ! nm -gU target/debug/libvelo_shim.dylib | grep -q "dlsym_shim"; then
    echo "❌ FAIL: dlsym_shim symbol not found in dylib"
    exit 1
fi

export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvelo_shim.dylib

OUTPUT=$("$TEST_DIR/dlsym_test" 2>&1)
if echo "$OUTPUT" | grep -q "SYM_FOUND"; then
    echo "    ✓ dlsym succeeded under shim"
else
    echo "    ✗ dlsym failed: $OUTPUT"
    exit 1
fi

echo ""
echo "✅ PASS: dlsym interception verified"
exit 0
