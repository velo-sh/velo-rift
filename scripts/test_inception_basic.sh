#!/bin/bash
# ============================================================================
# VRift Functional Test: Basic Shim Interception
# ============================================================================
set -e

safe_rm() {
    local target="$1"
    if [ -e "$target" ]; then
        if [ "$(uname -s)" == "Darwin" ]; then
            chflags -R nouchg "$target" 2>/dev/null || true
        else
            chattr -R -i "$target" 2>/dev/null || true
        fi
        rm -rf "$target"
    fi
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
VELO_BIN="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"

OS_TYPE=$(uname -s)
if [ "$OS_TYPE" == "Darwin" ]; then
    INCEPTION_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
else
    INCEPTION_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.so"
    PRELOAD_VAR="LD_PRELOAD"
fi

TEST_DIR=$(mktemp -d)
echo "Work Dir: $TEST_DIR"

cleanup() {
    pkill vriftd || true
    safe_rm "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT

echo "Testing Shim Basic Interception in $TEST_DIR"

# 1. Setup Workspace (RFC-0039 style)
mkdir -p "$TEST_DIR/source"
echo -n "hello world" > "$TEST_DIR/source/testfile.txt"

# 2. Ingest into the workspace's default .vrift/manifest.lmdb
echo "Ingesting source..."
export VR_THE_SOURCE="$TEST_DIR/cas"
# Use --prefix "/vrift" to match the shim's VFS prefix
# Standardize: ingest to .vrift/manifest.lmdb instead of custom file
# Ensure no stale daemon is running
pkill -f vriftd || true
rm -f /tmp/vrift.sock

# 3. Start daemon first
echo "Starting daemon..."
export VR_THE_SOURCE="$TEST_DIR/cas"
export RUST_LOG=info
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
VRIFTD_PID=$!
sleep 2

# 2. Ingest source (connects to the already running daemon)
echo "Ingesting source..."
export VRIFT_SOCKET_PATH="/tmp/vrift.sock"
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix "/vrift" > "$TEST_DIR/ingest.log" 2>&1
"$VELO_BIN" status --manifest "$TEST_DIR/source/.vrift/manifest.lmdb"
echo "CAS Physical Content:"
find "$TEST_DIR/cas" -type f
echo "VDir Physical Content (mmap Check):"
ls -lh "$HOME/.vrift/vdir" || true

# 4. Compile C test
cat > "$TEST_DIR/test.c" << 'CEOF'
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>

int main() {
    printf("[Test] Attempting to open /vrift/testfile.txt\n");
    int fd = open("/vrift/testfile.txt", O_RDONLY);
    if (fd < 0) {
        printf("[Test] Open failed with errno %d: %s\n", errno, strerror(errno));
        return 1;
    }
    char buf[12];
    int n = read(fd, buf, 11);
    if (n < 0) {
        printf("[Test] Read failed with errno %d\n", errno);
        return 1;
    }
    buf[n] = 0;
    printf("[Test] Read content: '%s'\n", buf);
    if (strcmp(buf, "hello world") != 0) return 1;
    close(fd);
    printf("[Test] Success!\n");
    return 0;
}
CEOF
gcc "$TEST_DIR/test.c" -o "$TEST_DIR/test"

# 5. Run with inception (Point VRIFT_PROJECT_ROOT to the ingested workspace)
echo "Running with inception..."
INCEPTION_PATH="$(realpath "$INCEPTION_LIB")"
if [ ! -f "$INCEPTION_PATH" ]; then
    echo "❌ ERROR: Shim library not found at $INCEPTION_PATH"
    exit 1
fi

export "$PRELOAD_VAR"="$INCEPTION_PATH"
if [ "$OS_TYPE" == "Darwin" ]; then
    export DYLD_FORCE_FLAT_NAMESPACE=1
fi
export VR_THE_SOURCE="$TEST_DIR/cas"
export VRIFT_MANIFEST="$TEST_DIR/source/.vrift/manifest.lmdb"
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_PROJECT_ROOT="$TEST_DIR/source"
export VRIFT_DEBUG=1

set +e
"$TEST_DIR/test" > "$TEST_DIR/test_output.log" 2>&1
set -e

if grep -q "Success!" "$TEST_DIR/test_output.log"; then
    echo "✅ Success: Shim intercepted correctly!"
else
    echo "❌ Failure: Shim test failed."
    cat "$TEST_DIR/test_output.log"
    echo "--- Daemon Log ---"
    cat "$TEST_DIR/daemon.log"
    exit 1
fi
