#!/bin/bash
set -e

# Setup a fake VFS directory on disk
REAL_FAKE_DIR="/tmp/vrift_fake_fchmod_$$"
mkdir -p "$REAL_FAKE_DIR"
touch "$REAL_FAKE_DIR/protected_file"
chmod 644 "$REAL_FAKE_DIR/protected_file"

# Build the POCs
cc tests/poc/test_chmod_shim.c -o tests/poc/test_chmod_shim
cc tests/poc/test_fchmod_gap.c -o tests/poc/test_fchmod_gap

export VRIFT_VFS_PREFIX="$REAL_FAKE_DIR"

echo "Using VRIFT_VFS_PREFIX=$VRIFT_VFS_PREFIX"

echo -e "\n1. Testing shimmed chmod (path-based C program):"
DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
./tests/poc/test_chmod_shim "$REAL_FAKE_DIR/protected_file"

# Verify permissions
current_mode=$(stat -f "%Lp" "$REAL_FAKE_DIR/protected_file")
echo "Current mode: $current_mode"
if [ "$current_mode" == "644" ]; then
    echo "FILE MODE UNCHANGED - OK (Chmod was blocked)"
else
    echo "BUG: FILE MODE CHANGED BY CHMOD! (Shim not working?)"
fi

echo -e "\n2. Testing UNSHIMMED fchmod (descriptor-based C program):"
DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
./tests/poc/test_fchmod_gap "$REAL_FAKE_DIR/protected_file"

final_mode=$(stat -f "%Lp" "$REAL_FAKE_DIR/protected_file")
echo "Final mode: $final_mode"
if [ "$final_mode" == "0" ]; then
    echo "GAP REPRODUCED: FILE MODE CHANGED BY FCHMOD BYPASS!"
else
    echo "FILE MODE UNCHANGED - UNEXPECTED"
fi

# Cleanup
rm -rf "$REAL_FAKE_DIR"
