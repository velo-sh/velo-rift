#!/bin/bash
# repro_manifest_key_mismatch.sh
# Verifies that manifest key lookup works correctly with virtual prefixes.

set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VELO_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

# Ensure vdir_d symlink for vDird subprocess model
VDIRD_BIN="$PROJECT_ROOT/target/release/vrift-vdird"
[ -f "$VDIRD_BIN" ] && [ ! -e "$(dirname "$VRIFTD_BIN")/vdir_d" ] && \
    ln -sf "vrift-vdird" "$(dirname "$VRIFTD_BIN")/vdir_d"

TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/source"
echo "manifest content" > "$TEST_DIR/source/foo.txt"

cleanup() {
    pkill -9 -f "$TEST_DIR" 2>/dev/null || true
    # Handle immutable CAS files
    if [ "$(uname -s)" = "Darwin" ]; then
        chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
    fi
    chmod -R +w "$TEST_DIR" 2>/dev/null || true
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT

# 1. Ingest with an explicit prefix
export VR_THE_SOURCE="$TEST_DIR/cas"
export VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock"
echo "--- Ingesting with prefix /myvirt ---"
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix /myvirt

# 2. Start daemon with isolated socket
export VRIFT_MANIFEST="$TEST_DIR/source/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$TEST_DIR/source"
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
VRIFTD_PID=$!
sleep 2

# 3. Proof: Try to access the file via shim
echo "--- Accessing /myvirt/foo.txt ---"
# Compile arm64 cat using open() (fopen may not trigger shim interception)
echo '#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        int fd = open(argv[i], O_RDONLY);
        if (fd < 0) { perror(argv[i]); return 1; }
        char buf[4096];
        ssize_t n;
        while ((n = read(fd, buf, sizeof(buf))) > 0)
            write(1, buf, n);
        close(fd);
    }
    return 0;
}' | cc -O2 -x c - -o "$TEST_DIR/cat"
codesign -s - -f "$TEST_DIR/cat" 2>/dev/null || true
codesign -s - -f "$SHIM_LIB" 2>/dev/null || true

set +e
OUTPUT=$(DYLD_INSERT_LIBRARIES="$SHIM_LIB" \
    DYLD_FORCE_FLAT_NAMESPACE=1 \
    VRIFT_VFS_PREFIX="/myvirt" \
    VRIFT_PROJECT_ROOT="$TEST_DIR/source" \
    VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock" \
    VRIFT_MANIFEST="$TEST_DIR/source/.vrift/manifest.lmdb" \
    VR_THE_SOURCE="$TEST_DIR/cas" \
    VRIFT_DEBUG=1 \
    "$TEST_DIR/cat" /myvirt/foo.txt 2>&1)
RET=$?
set -e

echo "$OUTPUT"

if echo "$OUTPUT" | grep -q "manifest content"; then
    echo "✅ PASS: Manifest key lookup succeeded for /myvirt/foo.txt"
    exit 0
else
    echo ""
    echo "--- Daemon Log ---"
    cat "$TEST_DIR/daemon.log"
    # If NOT FOUND, it's a known key mismatch issue
    if grep -q "NOT FOUND" "$TEST_DIR/daemon.log"; then
        echo "❌ FAIL: ManifestGet NOT FOUND — key mismatch"
        exit 1
    fi
    echo "❌ FAIL: File access failed"
    exit 1
fi
