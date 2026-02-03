#!/bin/bash
# Test: Shim Initialization Race Reproductive Step
# Purpose: Verify VFS interception works for the VERY FIRST syscall

set -e

# Setup paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../../" && pwd)"
VELO_BIN="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"

# OS Detection
if [ "$(uname -s)" == "Darwin" ]; then
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
else
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.so"
    PRELOAD_VAR="LD_PRELOAD"
fi

TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/source"
mkdir -p "$TEST_DIR/cas"

# Helper for cleaning up files that might be immutable (Solid hardlinks)
safe_rm() {
    local target="$1"
    if [ -e "$target" ]; then
        if [ "$(uname -s)" == "Darwin" ]; then
            chflags -R nouchg "$target" 2>/dev/null || true
        fi
        # Ignore permission errors - shim may be protecting files
        rm -rf "$target" 2>/dev/null || true
    fi
}

cleanup() {
    pkill -9 -f "$TEST_DIR" || true
    safe_rm "$TEST_DIR"
}
trap cleanup EXIT

echo "=== Repro Shell: Shim Init Race / Early Call ==="

# 1. Ingest
echo "secret content" > "$TEST_DIR/source/file.txt"
export VRIFT_CAS_ROOT="$TEST_DIR/cas"
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix "" -o "$TEST_DIR/source/vrift.manifest" > "$TEST_DIR/ingest.log" 2>&1

# 2. Start daemon with isolated socket
echo "Starting daemon..."
export VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock"
export VRIFT_MANIFEST="$TEST_DIR/source/vrift.manifest"
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
VRIFT_PID=$!
sleep 2

# 3. Reproductive Step: Try to cat the virtual file
cat <<EOF > "$TEST_DIR/tiny_cat.c"
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
int main(int argc, char** argv) {
    if (argc < 2) return 1;
    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }
    char buf[1024];
    ssize_t n;
    while ((n = read(fd, buf, sizeof(buf))) > 0) write(1, buf, n);
    close(fd);
    return 0;
}
EOF
gcc "$TEST_DIR/tiny_cat.c" -o "$TEST_DIR/cat"
codesign -s - -f "$TEST_DIR/cat" || true
codesign -s - -f "$SHIM_LIB" || true

echo "--- Proof Analysis ---"
set +e
# We expect success now because the race/deadlock is fixed.
# If it fails with "No such file", it means interception is NOT working.
env "$PRELOAD_VAR"="$SHIM_LIB" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock" \
VRIFT_VFS_PREFIX="/vrift" \
"$TEST_DIR/cat" /vrift/file.txt > "$TEST_DIR/output.txt" 2>&1
RET=$?
set -e

if grep -q "secret content" "$TEST_DIR/output.txt"; then
    echo "✅ SUCCESS: VFS Interception worked for early call!"
else
    echo "❌ FAILURE: VFS Interception failed."
    cat "$TEST_DIR/output.txt"
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

exit 0
