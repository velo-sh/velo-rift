#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
VELO_PROJECT_ROOT="$TEST_DIR/workspace"
DAEMON_BIN="${PROJECT_ROOT}/target/debug/vriftd"

echo "=== P0 Gap Test: Hard Link CAS Corruption Risk ==="

cat > "$TEST_DIR/link_test.c" << 'EOF'
#include <stdio.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>

int main(int argc, char *argv[]) {
    const char *src = argv[1];
    const char *dst = argv[2];
    
    printf("Linking %s -> %s\n", src, dst);
    if (link(src, dst) == 0) {
        printf("SUCCESS: Hard Link Created (CRITICAL FAIL)\n");
        return 0;
    } else {
        printf("FAILED: errno=%d (%s)\n", errno, strerror(errno));
        if (errno == 18 || errno == 1) { // EXDEV or EPERM
            printf("✅ PASS: Hard Link blocked\n");
            return 1;
        }
        return 2; // Other error
    }
}
EOF
gcc -o "$TEST_DIR/link_test" "$TEST_DIR/link_test.c"

# Prepare VFS
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "CAS_DATA" > "$VELO_PROJECT_ROOT/blobs.txt"
EXPORT_PATH="$TEST_DIR/export_link.txt"

# Start Daemon
rm -f /tmp/vrift.sock
(
    unset DYLD_INSERT_LIBRARIES
    unset LD_PRELOAD
    $DAEMON_BIN start > "$TEST_DIR/daemon.log" 2>&1 &
    echo $! > "$TEST_DIR/daemon.pid"
)
DAEMON_PID=$(cat "$TEST_DIR/daemon.pid")
sleep 2

# Register
mkdir -p "$HOME/.vrift/registry"
echo "{\"manifests\": {\"test_link\": {\"project_root\": \"$VELO_PROJECT_ROOT\"}}}" > "$HOME/.vrift/registry/manifests.json"
kill $DAEMON_PID || true
sleep 1
(
    unset DYLD_INSERT_LIBRARIES
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
export VRIFT_socket_path="/tmp/vrift.sock"

echo "Running link() test..."
set +e
(cd "$VELO_PROJECT_ROOT" && "$TEST_DIR/link_test" blobs.txt "$EXPORT_PATH")
RET=$?
set -e

kill $DAEMON_PID || true
rm -rf "$TEST_DIR"

if [[ $RET -eq 0 ]]; then
    echo "❌ FAIL: Protection Bypass. Hard link created directly to underlying storage."
    echo "   Risk: Modification of link corrupts CAS blob."
    exit 1
elif [[ $RET -eq 1 ]]; then
    echo "✅ PASS: Hard link blocked."
    exit 0
else
    echo "❌ FAIL: Unexpected error (shim active?)"
    exit 1
fi
