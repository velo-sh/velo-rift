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
$DAEMON_BIN start &
DAEMON_PID=$!
sleep 2

# Register workspace
mkdir -p "$HOME/.vrift/registry"
echo "{\"manifests\": {\"test_mmap\": {\"project_root\": \"$VELO_PROJECT_ROOT\"}}}" > "$HOME/.vrift/registry/manifests.json"
kill $DAEMON_PID
sleep 1
$DAEMON_BIN start &
DAEMON_PID=$!
sleep 2

echo "[3] Running functional test..."
export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
if [[ "$(uname)" == "Linux" ]]; then
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi
export VRIFT_socket_path="${VELO_PROJECT_ROOT}/.vrift/socket"

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
