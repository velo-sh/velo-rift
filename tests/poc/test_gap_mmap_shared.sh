#!/bin/bash
# RFC-0049 Gap Test: mmap(MAP_SHARED) tracking
#
# This is a P0 gap for Git pack and SQLite.
# Problem: writes via mmap don't go through write() shim.
# Mitigation: We track mmap regions and trigger reingest on munmap.

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P0 Gap Test: mmap(MAP_SHARED) Persistence ==="
echo ""

# Compile helper
cat > mmap_test.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <file>\n", argv[0]);
        return 1;
    }

    const char *path = argv[1];
    
    // 1. Open file (should trigger VFS CoW)
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        perror("open");
        return 1;
    }

    // 2. mmap (MAP_SHARED)
    // Map 4KB or file size
    size_t len = 4096;
    void *addr = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (addr == MAP_FAILED) {
        perror("mmap");
        return 1;
    }

    // 3. Write updates
    const char *msg = "UPDATED_BY_MMAP";
    memcpy(addr, msg, strlen(msg));

    // 4. Unmap (Should trigger reingest)
    if (munmap(addr, len) != 0) {
        perror("munmap");
        return 1;
    }

    // 5. Close
    close(fd);
    return 0;
}
EOF

gcc -o mmap_test mmap_test.c

echo "[1] Starting Daemon..."
export VELO_PROJECT_ROOT="${SCRIPT_DIR}/test_mmap_root"
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
rm -rf "$VELO_PROJECT_ROOT/.vrift/socket"
DAEMON_BIN="${PROJECT_ROOT}/target/debug/vriftd"
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    unset LD_PRELOAD
    $DAEMON_BIN start &
    echo $! > "$VELO_PROJECT_ROOT/daemon.pid"
)
DAEMON_PID=$(cat "$VELO_PROJECT_ROOT/daemon.pid")
sleep 2

# Register workspace
export VRIFT_REGISTRY_DIR="$TEST_DIR/registry"
mkdir -p "$VRIFT_REGISTRY_DIR"
echo "{\"version\": 1, \"manifests\": {\"test_mmap\": {\"source_path\": \"/tmp/test_mmap.manifest\", \"source_path_hash\": \"none\", \"project_root\": \"$VELO_PROJECT_ROOT\", \"registered_at\": \"2026-02-03T00:00:00Z\", \"last_verified\": \"2026-02-03T00:00:00Z\", \"status\": \"active\"}}}" > "$VRIFT_REGISTRY_DIR/manifests.json"
kill $DAEMON_PID || true
sleep 1
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    unset LD_PRELOAD
    $DAEMON_BIN start &
    echo $! > "$VELO_PROJECT_ROOT/daemon.pid"
)
DAEMON_PID=$(cat "$VELO_PROJECT_ROOT/daemon.pid")
sleep 2

echo "[3] Running functional test..."
if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi
export VRIFT_SOCKET_PATH="${VELO_PROJECT_ROOT}/.vrift/socket"

TEST_FILE="$VELO_PROJECT_ROOT/mapped_file.txt"
# Create initial file (size needs to be enough for mmap 4k or we handle SIGBUS?)
# Actually mmap extends file? No.
# Only trunc/ftruncate extends.
# We should create file with some size.
dd if=/dev/zero of="$TEST_FILE" bs=4096 count=1 > /dev/null 2>&1

./mmap_test "$TEST_FILE"

echo "[4] Verifying content..."
# Read first few bytes
CONTENT=$(head -c 15 "$TEST_FILE")
echo "File content: $CONTENT"

kill $DAEMON_PID
rm mmap_test mmap_test.c

if [[ "$CONTENT" == "UPDATED_BY_MMAP"* ]]; then
    echo "✅ PASS: mmap writes persisted via reingest"
    exit 0
else
    echo "❌ FAIL: Context mismatch. Wanted 'UPDATED_BY_MMAP', got '$CONTENT'"
    exit 1
fi
