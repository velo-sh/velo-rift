#!/bin/bash
set -e

# Setup a fake VFS directory on disk
REAL_FAKE_DIR="/tmp/vrift_fake_suite_$$"
mkdir -p "$REAL_FAKE_DIR"
touch "$REAL_FAKE_DIR/protected_file"
export VRIFT_VFS_PREFIX="$REAL_FAKE_DIR"

# Build all POCs
cc tests/poc/test_unlinkat_gap.c -o tests/poc/test_unlinkat_gap
cc tests/poc/test_mkdirat_gap.c -o tests/poc/test_mkdirat_gap
cc tests/poc/test_symlinkat_gap.c -o tests/poc/test_symlinkat_gap
cc tests/poc/test_fchmod_gap.c -o tests/poc/test_fchmod_gap
cc tests/poc/test_futimens_gap.c -o tests/poc/test_futimens_gap
cc tests/poc/test_sendfile_gap.c -o tests/poc/test_sendfile_gap

SHIM_SO="$(pwd)/target/debug/libvrift_shim.dylib"

echo "Using VRIFT_VFS_PREFIX=$VRIFT_VFS_PREFIX"

run_test() {
    local name=$1
    local cmd=$2
    echo -e "\n--- Testing $name ---"
    DYLD_INSERT_LIBRARIES="$SHIM_SO" \
    DYLD_FORCE_FLAT_NAMESPACE=1 \
    $cmd
}

run_test "unlinkat" "./tests/poc/test_unlinkat_gap $REAL_FAKE_DIR/protected_file"
# Recreate
touch "$REAL_FAKE_DIR/protected_file"

run_test "mkdirat" "./tests/poc/test_mkdirat_gap $REAL_FAKE_DIR/new_dir_at"
run_test "symlinkat" "./tests/poc/test_symlinkat_gap target $REAL_FAKE_DIR/link_at"
run_test "fchmod" "./tests/poc/test_fchmod_gap $REAL_FAKE_DIR/protected_file"
run_test "futimens" "./tests/poc/test_futimens_gap $REAL_FAKE_DIR/protected_file"

echo "hello" > "$REAL_FAKE_DIR/src_file"
run_test "sendfile" "./tests/poc/test_sendfile_gap $REAL_FAKE_DIR/src_file $REAL_FAKE_DIR/dest_file"

# Cleanup
rm -rf "$REAL_FAKE_DIR"
