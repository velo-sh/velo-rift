#!/bin/bash

# This script simulates a 'pnpm'-like layout with deep symlinking
# to verify Velo Rift's ability to handle high-density link trees.

echo "--- Node.js (pnpm-style) Layout Verification ---"

TEST_DIR=$(mktemp -d)
CAS_ROOT="$TEST_DIR/cas"
MANIFEST="$TEST_DIR/manifest.bin"
mkdir -p "$CAS_ROOT" "$TEST_DIR/virtual_node_modules/.pnpm"

# 1. Create a "Global Store" with actual content
mkdir -p "$TEST_DIR/store/react@18.0.0/node_modules/react"
echo "React Core Library" > "$TEST_DIR/store/react@18.0.0/node_modules/react/index.js"

# 2. Ingest the Global Store
echo "[+] Ingesting Global Store..."
./target/debug/vrift --the-source-root "$CAS_ROOT" ingest "$TEST_DIR/store" --mode solid --output "$MANIFEST" --prefix /store

# 3. Setup Virtual node_modules with symlinks
# .pnpm/react@18.0.0/node_modules/react -> /store/react@18.0.0/node_modules/react
ln -s "/store/react@18.0.0/node_modules/react" "$TEST_DIR/virtual_node_modules/.pnpm/react-link"

# 4. Projection via Shim
echo "[+] Verifying Symlink Resolution via Shim..."
export VRIFT_MANIFEST="$MANIFEST"
export VR_THE_SOURCE="$CAS_ROOT"
export VRIFT_VFS_PREFIX="/store" # The prefix we used during ingest
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="$(pwd)/target/debug/libvelo_shim.dylib"

# Attempt to read through the SYMLINK which points to a PROJECTED path
# Note: Since the shim only intercepts 'open', it should work IF the OS
# resolves the symlink and then the shim intercepts the resulting path.
TARGET_PATH="$TEST_DIR/virtual_node_modules/.pnpm/react-link/index.js"

if cat "$TARGET_PATH" 2>/dev/null | grep -q "React Core Library"; then
    echo "[SUCCESS] Symlink traversal to projected asset works."
else
    echo "[FAIL] Failed to read through symlink to projected asset."
    echo "       (Result likely empty or error: $(cat "$TARGET_PATH" 2>&1))"
fi

unset DYLD_INSERT_LIBRARIES
rm -rf "$TEST_DIR"
