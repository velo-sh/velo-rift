#!/bin/bash
# =============================================================================
# ABI Hazard Verification Test
# =============================================================================
# Validates that variadic syscalls (open, fcntl) correctly pass arguments
# on macOS ARM64. This is critical for DYLD_INSERT_LIBRARIES shims.
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Build shim
echo "Building vrift-shim..."
cargo build -p vrift-shim --release

# Compile test binary
TEST_BIN="/tmp/verify_abi_hazard"
echo "Compiling ABI hazard test..."
cc -o "$TEST_BIN" "$PROJECT_ROOT/tests/poc/verify_abi_hazard.c"

# Sign for macOS (required for DYLD_INSERT_LIBRARIES)
if [[ "$(uname)" == "Darwin" ]]; then
    codesign -s - -f "$TEST_BIN" 2>/dev/null || true
fi

# Run test with shim injected
echo "Running ABI hazard test with shim..."
DYLD_INSERT_LIBRARIES="$PROJECT_ROOT/target/release/libvrift_shim.dylib" \
LD_PRELOAD="$PROJECT_ROOT/target/release/libvrift_shim.so" \
"$TEST_BIN"

echo "✅ ABI Hazard Test Passed"
