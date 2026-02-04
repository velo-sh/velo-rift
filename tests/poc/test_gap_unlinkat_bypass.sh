#!/bin/bash
set -e

# Setup a fake VFS directory on disk
REAL_FAKE_DIR="/tmp/vrift_fake_$$"
mkdir -p "$REAL_FAKE_DIR"
touch "$REAL_FAKE_DIR/protected_file"

# Build the POCs
cc tests/poc/test_unlink_shim.c -o tests/poc/test_unlink_shim
cc tests/poc/test_unlinkat_gap.c -o tests/poc/test_unlinkat_gap

export VRIFT_VFS_PREFIX="$REAL_FAKE_DIR"

echo "Using VRIFT_VFS_PREFIX=$VRIFT_VFS_PREFIX"

echo -e "\n1. Testing shimmed unlink (C program):"
DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
./tests/poc/test_unlink_shim "$REAL_FAKE_DIR/protected_file"

if [ -f "$REAL_FAKE_DIR/protected_file" ]; then
    echo "FILE STILL EXISTS - OK (Unlink was likely blocked)"
else
    echo "BUG: FILE DELETED BY UNLINK! (Shim not working?)"
fi

echo -e "\n2. Testing UNSHIMMED unlinkat (C program):"
if [ ! -f "$REAL_FAKE_DIR/protected_file" ]; then
    touch "$REAL_FAKE_DIR/protected_file"
fi

DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
./tests/poc/test_unlinkat_gap "$REAL_FAKE_DIR/protected_file"

if [ -f "$REAL_FAKE_DIR/protected_file" ]; then
    echo "FILE STILL EXISTS - UNEXPECTED (Gap not reproduced?)"
else
    echo "GAP REPRODUCED: FILE DELETED BY UNLINKAT BYPASS!"
fi

# Cleanup
rm -rf "$REAL_FAKE_DIR"
