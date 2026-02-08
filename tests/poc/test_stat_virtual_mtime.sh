#!/bin/bash
# Test: stat Virtual Metadata - Runtime Verification
# Purpose: Verify device ID and virtual metadata at runtime
# Priority: P0

set -e

# Setup paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../../" && pwd)"
VELO_BIN="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"
SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"

# Ensure vdir_d symlink for vDird subprocess model
VDIRD_BIN="${PROJECT_ROOT}/target/release/vrift-vdird"
[ -f "$VDIRD_BIN" ] && [ ! -e "$(dirname "$VRIFTD_BIN")/vdir_d" ] && \
    ln -sf "vrift-vdird" "$(dirname "$VRIFTD_BIN")/vdir_d"

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
        rm -rf "$target"
    fi
}

cleanup() {
    pkill -9 -f "$TEST_DIR" || true
    safe_rm "$TEST_DIR"
}
trap cleanup EXIT

echo "=== Test: stat Virtual Metadata (Runtime) ==="

# 1. Ingest
echo "Ingesting source..."
export VR_THE_SOURCE="$TEST_DIR/cas"
echo -n "test content" > "$TEST_DIR/source/test_file.txt"
# Use --prefix "/vrift" to match VRIFT_VFS_PREFIX so manifest keys align
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix "/vrift" > "$TEST_DIR/ingest.log" 2>&1

# 2. Start daemon with isolated socket
echo "Starting daemon..."
export VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock"
# Use LMDB manifest from ingest output
export VRIFT_MANIFEST="$TEST_DIR/source/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$TEST_DIR/source"
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
VRIFTD_PID=$!
sleep 2

# 3. Compile helper C stat test program
echo "Compiling C stat test program..."
cat > "$TEST_DIR/test.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    struct stat sb;
    if (stat(argv[1], &sb) != 0) { perror("stat"); return 1; }
    printf("dev=0x%llx size=%lld\n", (unsigned long long)sb.st_dev, (long long)sb.st_size);
    // RIFT device ID = 0x52494654
    if (sb.st_dev == 0x52494654) {
        printf("✅ PASS: VFS device ID detected (RIFT)\n");
        return 0;
    } else {
        printf("❌ FAIL: Not VFS device (expected 0x52494654)\n");
        return 1;
    }
}
EOF
gcc "$TEST_DIR/test.c" -o "$TEST_DIR/test_stat"
codesign -v -s - -f "$TEST_DIR/test_stat" || true
codesign -v -s - -f "$SHIM_LIB" || true

# 4. Run with shim
echo "Running with shim..."
set +e
DYLD_INSERT_LIBRARIES="$SHIM_LIB" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock" \
VRIFT_PROJECT_ROOT="$TEST_DIR/source" \
VRIFT_VFS_PREFIX="/vrift" \
VR_THE_SOURCE="$TEST_DIR/cas" \
VRIFT_DEBUG=1 \
"$TEST_DIR/test_stat" "/vrift/test_file.txt" > "$TEST_DIR/test_output.log" 2>&1
RET=$?
set -e

if grep -q "PASS: VFS device ID detected (RIFT)" "$TEST_DIR/test_output.log"; then
    echo "✅ Success: stat virtual metadata verified!"
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
