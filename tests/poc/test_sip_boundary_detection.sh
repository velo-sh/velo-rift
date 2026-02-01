#!/bin/bash
# QA Test: SIP Boundary Detection
# Documents which binaries can/cannot be intercepted by DYLD_INSERT_LIBRARIES
# This is a fundamental macOS limitation that affects the shim architecture

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"

echo "=== QA Test: macOS SIP Boundary Detection ==="
echo "Determines which binaries can be shimmed vs SIP-protected"
echo ""

if [[ ! -f "$SHIM_PATH" ]]; then
    echo "❌ SKIP: Shim not built"
    exit 0
fi

export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1

SIP_COUNT=0
SHIM_COUNT=0

test_binary() {
    local name="$1"
    local cmd="$2"
    
    OUTPUT=$(DYLD_PRINT_LIBRARIES=1 $cmd 2>&1 | head -20)
    if echo "$OUTPUT" | grep -q "libvrift_shim"; then
        echo "✅ $name: Shim LOADED"
        ((SHIM_COUNT++))
    else
        echo "❌ $name: SIP BLOCKED"
        ((SIP_COUNT++))
    fi
}

echo "=== System Binaries (expected: SIP protected) ==="
test_binary "/bin/chmod" "/bin/chmod --version"
test_binary "/bin/rm" "/bin/rm --version"
test_binary "/bin/ln" "/bin/ln --version"
test_binary "/bin/cp" "/bin/cp --version"
test_binary "/bin/mv" "/bin/mv --version"

echo ""
echo "=== User Binaries (expected: can shim) ==="

if command -v rustc &> /dev/null; then
    test_binary "rustc" "rustc --version"
fi

if command -v cargo &> /dev/null; then
    test_binary "cargo" "cargo --version"
fi

if command -v node &> /dev/null; then
    test_binary "node" "node --version"
fi

unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE

echo ""
echo "=== Summary ==="
echo "SIP Protected: $SIP_COUNT"
echo "Can Shim: $SHIM_COUNT"
echo ""
echo "IMPLICATION: Build scripts using /bin/* commands will bypass shim"

# Always pass - this is a documentation test
exit 0
