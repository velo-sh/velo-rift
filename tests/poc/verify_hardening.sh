#!/bin/bash
# RFC-0054: Automated Hardening Verification Test
# Validates that metadata mutations inside VFS territory are correctly blocked.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BIN_PATH="$SCRIPT_DIR/verify_hardening"
SHIM_PATH="$REPO_ROOT/target/debug/libvrift_inception_layer.dylib"

# 1. Build the POC
echo "--- Building Hardening POC ---"
gcc -o "$BIN_PATH" "$SCRIPT_DIR/verify_hardening.c"

# 2. Build the Shim
echo "--- Building Velo Rift Shim ---"
cargo build -p vrift-inception-layer

# 3. Setup Test Environment
export VRIFT_VFS_PREFIX="/Users/antigravity/vrift_vfs"
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export VRIFT_DEBUG=1

echo "--- Running Hardening Verification ---"
# Trigger the test
if "$BIN_PATH"; then
    echo "✅ PASS: Hardening verification successful"
else
    echo "❌ FAIL: Hardening verification failed"
    exit 1
fi

# Cleanup binary
rm "$BIN_PATH"
