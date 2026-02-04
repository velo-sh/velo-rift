#!/bin/bash
set -e

# Setup a fake VFS directory on disk
REAL_FAKE_DIR="/tmp/vrift_fake_mkdir_$$"
mkdir -p "$REAL_FAKE_DIR"

# Build the POCs
cc tests/poc/test_mkdir_shim.c -o tests/poc/test_mkdir_shim
cc tests/poc/test_mkdirat_gap.c -o tests/poc/test_mkdirat_gap

export VRIFT_VFS_PREFIX="$REAL_FAKE_DIR"

echo "Using VRIFT_VFS_PREFIX=$VRIFT_VFS_PREFIX"

echo -e "\n1. Testing shimmed mkdir (C program):"
DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
./tests/poc/test_mkdir_shim "$REAL_FAKE_DIR/new_dir"

if [ ! -d "$REAL_FAKE_DIR/new_dir" ]; then
    echo "DIR DOES NOT EXIST - OK (Mkdir was blocked)"
else
    echo "BUG: DIR CREATED BY MKDIR! (Shim not working?)"
fi

echo -e "\n2. Testing UNSHIMMED mkdirat (C program):"
DYLD_INSERT_LIBRARIES=$(pwd)/target/debug/libvrift_shim.dylib \
DYLD_FORCE_FLAT_NAMESPACE=1 \
./tests/poc/test_mkdirat_gap "$REAL_FAKE_DIR/new_dir_at"

if [ -d "$REAL_FAKE_DIR/new_dir_at" ]; then
    echo "GAP REPRODUCED: DIR CREATED BY MKDIRAT BYPASS!"
else
    echo "DIR DOES NOT EXIST - UNEXPECTED"
fi

# Cleanup
rm -rf "$REAL_FAKE_DIR"
