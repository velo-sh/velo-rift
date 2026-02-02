#!/bin/bash
# ============================================================================
# VRift Functional Test: Basic Shim Interception
# ============================================================================
# Verifies that the shim correctly intercepts open/read for virtual paths
# via the daemon IPC.
# ============================================================================

set -e

# Helper for cleaning up files that might be immutable (Solid hardlinks)
safe_rm() {
    local target="$1"
    if [ -e "$target" ]; then
        if [ "$(uname -s)" == "Darwin" ]; then
            chflags -R nouchg "$target" 2>/dev/null || true
        else
            # Try chattr -i on Linux if available
            chattr -R -i "$target" 2>/dev/null || true
        fi
        rm -rf "$target"
    fi
}

# Setup paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
VELO_BIN="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"

# Detect OS
OS_TYPE=$(uname -s)
if [ "$OS_TYPE" == "Darwin" ]; then
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
else
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.so"
    PRELOAD_VAR="LD_PRELOAD"
fi

# Ensure binaries exist
if [ ! -f "$VELO_BIN" ] || [ ! -f "$VRIFTD_BIN" ] || [ ! -f "$SHIM_LIB" ]; then
    echo "Building release binaries..."
    cargo build --release --workspace
    # Explicitly build shim cdylib (may not be built by --workspace alone)
    cargo build --release -p vrift-shim
fi

TEST_DIR=$(mktemp -d)
echo "Work Dir: $TEST_DIR"

cleanup() {
    echo "Cleaning up..."
    pkill vriftd || true
    safe_rm "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT

echo "Testing Shim Basic Interception in $TEST_DIR"

mkdir -p "$TEST_DIR/source"
echo -n "hello world" > "$TEST_DIR/source/testfile.txt"

# Ensure clean environment
unset VRIFT_PROJECT_ROOT
unset VRIFT_INCEPTION
unset VRIFT_SOCKET_PATH
unset VRIFT_CAS_ROOT
unset VRIFT_MANIFEST
unset VRIFT_VFS_PREFIX

# 1. Ingest
echo "Ingesting source..."
export VRIFT_CAS_ROOT="$TEST_DIR/cas"
# Use --prefix "" for correct /testfile.txt mapping
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix "" -o "$TEST_DIR/source/vrift.manifest" > "$TEST_DIR/ingest.log" 2>&1

# 2. Start daemon
echo "Starting daemon..."
export VRIFT_CAS_ROOT="$TEST_DIR/cas"
export RUST_LOG=info

"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
VRIFTD_PID=$!
sleep 2

# Verify daemon is running
if ! pgrep vriftd > /dev/null; then
    echo "❌ ERROR: Daemon died immediately."
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

# 3. Compile helper C test
# ... (lines 100-134 omitted for brevity, but I'll keep the block intact)
echo "Compiling C test program..."
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
        printf("[Test] Read failed with errno %d: %s\n", errno, strerror(errno));
        return 1;
    }
    buf[n] = 0;
    printf("[Test] Read content: '%s'\n", buf);
    if (strcmp(buf, "hello world") != 0) {
        printf("[Test] Content mismatch: expected 'hello world', got '%s'\n", buf);
        return 1;
    }
    close(fd);
    printf("[Test] Success!\n");
    return 0;
}
CEOF
gcc "$TEST_DIR/test.c" -o "$TEST_DIR/test"

# 4. Run with shim
echo "Running with shim..."
SHIM_PATH="$(realpath "$SHIM_LIB")"
if [ ! -f "$SHIM_PATH" ]; then
    echo "❌ ERROR: Shim library not found at $SHIM_PATH"
    exit 1
fi

export "$PRELOAD_VAR"="$SHIM_PATH"
if [ "$OS_TYPE" == "Darwin" ]; then
    export DYLD_FORCE_FLAT_NAMESPACE=1
fi
export VRIFT_CAS_ROOT="$TEST_DIR/cas"
export VRIFT_MANIFEST="$TEST_DIR/source/vrift.manifest"
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_DEBUG=1

# Capture output
STRACE_CMD=""
# Disabled automatic strace to prevent hangs in CI/non-interactive modes
# if command -v strace >/dev/null 2>&1; then
#     STRACE_CMD="strace -f -e trace=file,open,openat,openat2,stat,lstat,fstat,newfstatat,fstatat64"
# fi
if [ -n "$VRIFT_STRACE" ]; then
    STRACE_CMD="dtruss" 
fi

echo "Running with strace: $STRACE_CMD"
set +e
if [ -n "$STRACE_CMD" ]; then
    # Run with strace and capture its output to stderr (which we redirect anyway)
    export "$PRELOAD_VAR"="$SHIM_PATH"
    $STRACE_CMD "$TEST_DIR/test" > "$TEST_DIR/test_output.log" 2>&1
else
    export "$PRELOAD_VAR"="$SHIM_PATH"
    "$TEST_DIR/test" > "$TEST_DIR/test_output.log" 2>&1
fi
set -e

if grep -q "Success!" "$TEST_DIR/test_output.log"; then
    echo "✅ Success: Shim intercepted and returned correct data!"
    cat "$TEST_DIR/test_output.log"
else
    echo "❌ Failure: Shim test failed."
    echo "--- Test Output ---"
    cat "$TEST_DIR/test_output.log"
    echo "--- Daemon Log ---"
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

exit 0
