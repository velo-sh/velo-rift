#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
VELO_PROJECT_ROOT="$TEST_DIR/workspace"
DAEMON_BIN="${PROJECT_ROOT}/target/debug/vriftd"

echo "=== P0 Gap Test: VFS Boundary Export (rename EXDEV) ==="

# Helper for C program
cat > "$TEST_DIR/boundary_test.c" << 'EOF'
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
    if (argc < 3) return 1;
    const char *src = argv[1];
    const char *dst = argv[2];
    
    printf("Renaming %s -> %s\n", src, dst);
    if (rename(src, dst) == 0) {
        printf("SUCCESS: rename succeeded (PASSTHROUGH DETECTED)\n");
        return 0;
    } else {
        if (errno == 18) { // EXDEV
            printf("✅ CONFIRMED: Got EXDEV (Cross-device link)\n");
            return 2; // Special code for Success
        } else {
            printf("FAILED: errno=%d (%s)\n", errno, strerror(errno));
            return 1;
        }
    }
}
EOF
gcc -o "$TEST_DIR/boundary_test" "$TEST_DIR/boundary_test.c"

# Prepare
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "Hello boundary" > "$VELO_PROJECT_ROOT/test.txt"
EXTERNAL_DIR="$TEST_DIR/external"
mkdir -p "$EXTERNAL_DIR"
export VRIFT_socket_path="/tmp/vrift.sock"

# Start Daemon
rm -f /tmp/vrift.sock
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    unset LD_PRELOAD
    $DAEMON_BIN start > "$TEST_DIR/daemon.log" 2>&1 &
    echo $! > "$TEST_DIR/daemon.pid"
)
DAEMON_PID=$(cat "$TEST_DIR/daemon.pid")
sleep 2

if ! ps -p $DAEMON_PID > /dev/null; then
    echo "❌ Daemon failed to start. Log:"
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

# Register
mkdir -p "$HOME/.vrift/registry"
echo "{\"manifests\": {\"test_boundary\": {\"project_root\": \"$VELO_PROJECT_ROOT\"}}}" > "$HOME/.vrift/registry/manifests.json"
kill $DAEMON_PID || true
sleep 1
rm -f /tmp/vrift.sock
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    unset LD_PRELOAD
    $DAEMON_BIN start >> "$TEST_DIR/daemon.log" 2>&1 &
    echo $! > "$TEST_DIR/daemon.pid"
)
DAEMON_PID=$(cat "$TEST_DIR/daemon.pid")
sleep 2

# Setup Shim
export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="$LD_PRELOAD"
    export DYLD_FORCE_FLAT_NAMESPACE=1
fi
export RUST_LOG=debug

echo "Running rename..."
set +e
(cd "$VELO_PROJECT_ROOT" && "$TEST_DIR/boundary_test" test.txt "$EXTERNAL_DIR/test.txt")
RET=$?
set -e

kill $DAEMON_PID || true
rm -rf "$TEST_DIR"

if [[ $RET -eq 2 ]]; then
    echo "✅ PASS: VFS boundary enforced (EXDEV returned)"
    exit 0
elif [[ $RET -eq 0 ]]; then
    echo "❌ FAIL: Shim Bypass Detected (rename succeeded via OS)"
    echo "Impact: VFS metadata state drift (Manifest stale)"
    exit 1
else
    echo "❌ FAIL: Unexpected error"
    exit 1
fi
