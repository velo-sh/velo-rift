#!/bin/bash
# ============================================================================
# Secondary Mutation Protection Verification
# ============================================================================
# Verifies that Velo Rift blocks timestamp and attribute mutations on managed files.
# Syscalls verified: futimes, fchflags (macOS), sendfile (macOS)

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
C_TEST_SRC="$PROJECT_ROOT/tests/qa_v2/test_secondary_mutation.c"

# Platform detection
OS=$(uname -s)
if [ "$OS" == "Darwin" ]; then
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
    VFS_ENV="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1"
else
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.so"
    VFS_ENV="LD_PRELOAD=$SHIM_LIB"
fi

# Color helpers
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

echo "----------------------------------------------------------------"
echo -e "${BLUE}🛡️  Velo Rift: Secondary Mutation Protection Test${NC}"
echo "----------------------------------------------------------------"

# Setup work dir
WORK_DIR="/tmp/vrift_secondary_mutation"
if [ "$OS" == "Darwin" ]; then
    chflags -R nouchg "$WORK_DIR" 2>/dev/null || true
fi
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/vfs_source"
mkdir -p "$WORK_DIR/cas"
mkdir -p "$WORK_DIR/bin"

# 1. Compile the test program
echo "[*] Compiling test binary..."
cc -O2 -o "$WORK_DIR/bin/test_secondary" "$C_TEST_SRC"

# 2. Setup VFS environment
echo "[*] Initializing VFS environment..."
TEST_FILE="$WORK_DIR/vfs_source/managed.txt"
echo "Protected Content" > "$TEST_FILE"
MANIFEST="$WORK_DIR/manifest.lmdb"

export VR_THE_SOURCE="$WORK_DIR/cas"
"$VRIFT_BIN" ingest "$WORK_DIR/vfs_source" --prefix /secondary_test --output "$MANIFEST" >/dev/null

VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"

# Start daemon
pkill vriftd 2>/dev/null || true
DEVICE_ID=1 # Standard test ID
export VRIFT_MANIFEST="$MANIFEST"
"$VRIFTD_BIN" start > "$WORK_DIR/daemon.log" 2>&1 &
sleep 2

# Cleanup on exit
trap "pkill vriftd || true" EXIT

# 3. Run Verification
FULL_VFS_ENV="VRIFT_MANIFEST=$MANIFEST VRIFT_VFS_PREFIX=/secondary_test $VFS_ENV"
VFS_PATH="/secondary_test/managed.txt"

run_test() {
    local type="$1"
    echo -e "\n${BLUE}🧪 Testing $type on $VFS_PATH...${NC}"
    if env $FULL_VFS_ENV "$WORK_DIR/bin/test_secondary" "$type" "$VFS_PATH"; then
        echo -e "   ${GREEN}✅ Success: $type was correctly blocked.${NC}"
    else
        echo -e "   ${RED}❌ Failed: $type was NOT blocked or crashed.${NC}"
        exit 1
    fi
}

# Always test futimes
run_test "futimes"

# macOS specific tests
if [ "$OS" == "Darwin" ]; then
    run_test "fchflags"
    run_test "sendfile"
fi

echo -e "\n${GREEN}✨ All secondary mutation protections verified!${NC}"
