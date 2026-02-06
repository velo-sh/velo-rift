#!/bin/bash
# Test: Bootstrap Deadlock Prevention (BUG-007)
#
# This test verifies that the shim can be loaded via DYLD_INSERT_LIBRARIES
# without causing a deadlock during the dyld bootstrap phase.
#
# The deadlock previously occurred because:
# 1. fstat is called inside __malloc_init before malloc is ready
# 2. fstat_shim used dlsym which needs malloc -> infinite recursion
#
# Solution: Use raw syscalls during early init (INITIALIZING >= 2)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

echo "=== BUG-007: Bootstrap Deadlock Test ==="

# Check platform
if [[ "$(uname)" != "Darwin" ]]; then
    echo -e "${YELLOW}SKIP: This test is macOS-specific${NC}"
    exit 0
fi

# Build release shim if needed
if [[ ! -f "$SHIM_LIB" ]]; then
    echo "Building release shim..."
    cargo build --release -p vrift-inception-layer 2>/dev/null
fi

if [[ ! -f "$SHIM_LIB" ]]; then
    echo -e "${RED}FAIL: Could not build shim${NC}"
    exit 1
fi

# Create test binary
TEST_BIN="/tmp/test_bug007_$$"
cat > "${TEST_BIN}.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <stdlib.h>

int main() {
    // This will trigger fstat during initialization
    struct stat sb;
    if (fstat(1, &sb) < 0) {
        // Ignore error, just testing we don't deadlock
    }
    printf("BUG007_OK\n");
    return 0;
}
EOF

cc -o "$TEST_BIN" "${TEST_BIN}.c" 2>/dev/null
codesign -f -s - "$TEST_BIN" 2>/dev/null || true
rm -f "${TEST_BIN}.c"

# Test 1: Normal injection (two-level namespace)
echo -n "Test 1: Two-level namespace... "
RESULT=$(DYLD_INSERT_LIBRARIES="$SHIM_LIB" "$TEST_BIN" 2>&1) 
if echo "$RESULT" | grep -q "BUG007_OK"; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL: Deadlock or crash detected${NC}"
    echo "Output: $RESULT"
    rm -f "$TEST_BIN"
    exit 1
fi

# Test 2: Flat namespace (more aggressive, can expose recursion issues)
echo -n "Test 2: Flat namespace... "
RESULT=$(DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 "$TEST_BIN" 2>&1)
if echo "$RESULT" | grep -q "BUG007_OK"; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL: Deadlock in flat namespace${NC}"
    echo "Output: $RESULT"
    rm -f "$TEST_BIN"
    exit 1
fi

# Test 3: Multiple rapid launches (stress test)
echo -n "Test 3: Rapid launch stress test (10x)... "
PASS_COUNT=0
for i in $(seq 1 10); do
    RESULT=$(DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 "$TEST_BIN" 2>&1)
    if echo "$RESULT" | grep -q "BUG007_OK"; then
        PASS_COUNT=$((PASS_COUNT + 1))
    fi
done
if [[ $PASS_COUNT -eq 10 ]]; then
    echo -e "${GREEN}PASS ($PASS_COUNT/10)${NC}"
else
    echo -e "${RED}FAIL: Only $PASS_COUNT/10 successful${NC}"
    rm -f "$TEST_BIN"
    exit 1
fi

# Cleanup
rm -f "$TEST_BIN"

echo ""
echo -e "${GREEN}All BUG-007 bootstrap tests passed!${NC}"
exit 0
