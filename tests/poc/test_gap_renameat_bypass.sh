#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
# Use realpath to resolve /var -> /private/var symlink on macOS
VELO_PROJECT_ROOT="$(cd "$TEST_DIR" && pwd -P)/workspace"
DAEMON_BIN="${PROJECT_ROOT}/target/debug/vriftd"

echo "=== P0 Gap Test: renameat() Bypass ==="

cat > "$TEST_DIR/renameat_test.c" << 'EOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>

int main(int argc, char *argv[]) {
    const char *src = argv[1];
    const char *dst = argv[2];
    
    // Use AT_FDCWD (-2)
    printf("renameat(AT_FDCWD, %s, AT_FDCWD, %s)\n", src, dst);
    if (renameat(-2, src, -2, dst) == 0) {
        printf("SUCCESS: renameat succeeded (PASSTHROUGH DETECTED)\n");
        return 0;
    } else {
        if (errno == 18) { // EXDEV
            printf("✅ CONFIRMED: Got EXDEV (Intercepted)\n");
            return 2;
        }
        printf("FAILED: errno=%d (%s)\n", errno, strerror(errno));
        return 1;
    }
}
EOF
gcc -o "$TEST_DIR/renameat_test" "$TEST_DIR/renameat_test.c"

# Prepare
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "Move me" > "$VELO_PROJECT_ROOT/test.txt"
EXPORT_PATH="$TEST_DIR/test_moved.txt"

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

export VRIFT_REGISTRY_DIR="$TEST_DIR/registry"
mkdir -p "$VRIFT_REGISTRY_DIR"
echo "{\"version\": 1, \"manifests\": {\"test_renameat\": {\"source_path\": \"/tmp/test_renameat.manifest\", \"source_path_hash\": \"none\", \"project_root\": \"$VELO_PROJECT_ROOT\", \"registered_at\": \"2026-02-03T00:00:00Z\", \"last_verified\": \"2026-02-03T00:00:00Z\", \"status\": \"active\"}}}" > "$VRIFT_REGISTRY_DIR/manifests.json"
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

if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi
export VRIFT_SOCKET_PATH="/tmp/vrift.sock"
export VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT"

echo "Running renameat test..."
set +e
(cd "$VELO_PROJECT_ROOT" && "$TEST_DIR/renameat_test" test.txt "$EXPORT_PATH")
RET=$?
set -e

kill $DAEMON_PID || true
rm -rf "$TEST_DIR"

if [[ $RET -eq 2 ]]; then
    echo "✅ PASS: renameat intercepted"
    exit 0
elif [[ $RET -eq 0 ]]; then
    echo "❌ FAIL: Shim Bypass Detected (renameat succeeded via OS)"
    exit 1
else
    echo "❌ FAIL: Unexpected error"
    exit 1
fi
