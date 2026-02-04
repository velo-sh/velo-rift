#!/bin/bash
# ============================================================================
# Test: Rename Syscall Interface (RFC-0047 compliance)
# ============================================================================
# This test verifies the shim's behavior at the syscall/libc level.
# Key expectations:
# 1. Outside -> VFS: rename() returns -1, errno=EXDEV (18)
# 2. VFS -> Outside: rename() returns -1, errno=EXDEV (18)
# 3. VFS -> VFS: rename() returns 0 (Succeeds)

set -e
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
[ ! -f "$SHIM_LIB" ] && SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.so"

WORK_DIR="/tmp/vrift_rename_interface"
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/project"
mkdir -p "$WORK_DIR/external"

# 1. Compile Rename Probe
cat > "$WORK_DIR/rename_probe.c" << 'EOF'
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "Usage: %s <old> <new>\n", argv[0]);
        return 2;
    }
    int ret = rename(argv[1], argv[2]);
    if (ret == 0) {
        printf("SUCCESS\n");
        return 0;
    } else {
        printf("FAILED errno=%d (%s)\n", errno, strerror(errno));
        return 0; // Exit 0 so script can parse output
    }
}
EOF
cc -O2 -o "$WORK_DIR/probe" "$WORK_DIR/rename_probe.c"
[ "$(uname -s)" == "Darwin" ] && codesign -s - -f "$WORK_DIR/probe"

# 2. Initialize VFS
cd "$WORK_DIR/project"
"$VRIFT_BIN" init . >/dev/null 2>&1
export VRIFT_VFS_PREFIX="$WORK_DIR/project"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1
export LD_PRELOAD="$SHIM_LIB"

# Files setup
echo "data" > "$WORK_DIR/external/ext.txt"
echo "data" > "$WORK_DIR/project/vfs.txt"

echo "----------------------------------------------------------------"
echo "🔍 Testing Rename Interface Behavior"
echo "----------------------------------------------------------------"

# Case 1: Outside -> VFS (Cross-boundary)
echo -n "Test 1: Outside -> VFS ... "
RES=$("$WORK_DIR/probe" "$WORK_DIR/external/ext.txt" "$WORK_DIR/project/ext_in.txt")
if [[ "$RES" == *"FAILED errno=18"* ]]; then
    echo "✅ PASS (errno=EXDEV)"
else
    echo "❌ FAIL ($RES)"
    exit 1
fi

# Case 2: VFS -> Outside (Cross-boundary)
echo -n "Test 2: VFS -> Outside ... "
RES=$("$WORK_DIR/probe" "$WORK_DIR/project/vfs.txt" "$WORK_DIR/external/vfs_out.txt")
if [[ "$RES" == *"FAILED errno=18"* ]]; then
    echo "✅ PASS (errno=EXDEV)"
else
    echo "❌ FAIL ($RES)"
    exit 1
fi

# Case 3: VFS -> VFS (Internal - Needs Daemon)
echo -n "Test 3: VFS -> VFS ... "
# Start daemon
export VR_THE_SOURCE="$WORK_DIR/cas"
mkdir -p "$VR_THE_SOURCE"
$PROJECT_ROOT/target/release/vriftd start &
DAEMON_PID=$!
export VRIFT_DAEMON_SOCKET="/tmp/vrift_test.sock"
sleep 1 # Wait for daemon to start

RES=$("$WORK_DIR/probe" "$WORK_DIR/project/vfs.txt" "$WORK_DIR/project/vfs_renamed.txt")

# Cleanup daemon
kill $DAEMON_PID 2>/dev/null || true
rm -f /tmp/vrift_test.sock

if [[ "$RES" == "SUCCESS" ]]; then
    echo "✅ PASS (Success)"
else
    echo "❌ FAIL ($RES)"
    exit 1
fi

echo "----------------------------------------------------------------"
echo "🏆 INTERFACE TEST: SUCCESSFUL"
echo "----------------------------------------------------------------"
rm -rf "$WORK_DIR"
