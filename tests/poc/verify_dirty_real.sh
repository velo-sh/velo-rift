#!/bin/bash
# Test: Dirty Consistency (Real vriftd) - Final Fix
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR="${PROJECT_ROOT}/test_dirty_final"
# CAS blobs may have immutable flags — must strip before rm
chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
chmod -R u+w "$TEST_DIR" 2>/dev/null || true
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
VELO_PROJECT_ROOT="$TEST_DIR/workspace"
mkdir -p "$VELO_PROJECT_ROOT"

echo "=== Test: Dirty Consistency (Real vriftd) ==="

# 1. Paths
VELO_BIN="${PROJECT_ROOT}/target/debug/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/debug/vriftd"
SHIM_BIN="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"

# Ensure vdir_d symlink for vDird subprocess model
VDIRD_BIN="${PROJECT_ROOT}/target/debug/vrift-vdird"
[ -f "$VDIRD_BIN" ] && [ ! -e "$(dirname "$VRIFTD_BIN")/vdir_d" ] && \
    ln -sf "vrift-vdird" "$(dirname "$VRIFTD_BIN")/vdir_d"

# 2. Preparation
echo "original content for test_file.txt" > "$VELO_PROJECT_ROOT/test_file.txt"
mkdir -p "$TEST_DIR/cas"
export VR_THE_SOURCE="$(realpath "$TEST_DIR/cas")"

echo "[Ingest] Ingesting workspace..."
# IMPORTANT: Put manifest inside workspace so project_root matches vfs_prefix
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
"$VELO_BIN" ingest "$VELO_PROJECT_ROOT" --output "$VELO_PROJECT_ROOT/.vrift/manifest.lmdb"

# 3. Start vriftd
pkill vriftd 2>/dev/null || true
sleep 1
export VRIFT_MANIFEST="$(realpath "$VELO_PROJECT_ROOT/.vrift/manifest.lmdb")"
export RUST_LOG=debug
echo "[Daemon] Starting vriftd with manifest $VRIFT_MANIFEST..."
"$VRIFTD_BIN" > "$TEST_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!

cleanup() {
    kill $DAEMON_PID 2>/dev/null || true
    # CAS blobs have immutable flags — strip before rm
    chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
    chmod -R u+w "$TEST_DIR" 2>/dev/null || true
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Wait for socket
MAX_RETRIES=10
RETRY_COUNT=0
while [ ! -S "/tmp/vrift.sock" ] && [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
    sleep 0.5
    RETRY_COUNT=$((RETRY_COUNT + 1))
done

if [ ! -S "/tmp/vrift.sock" ]; then
    echo "ERROR: Daemon failed to start."
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

# 4. Compile test program
cat > "$TEST_DIR/consistency_test.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>

void print_stat(const char* label, const char* path) {
    struct stat sb;
    if (stat(path, &sb) != 0) {
        printf("[%s] stat FAILED: %s (errno=%d)\n", label, strerror(errno), errno);
        return;
    }
    // Note: On macOS, dev is 32-bit in struct stat, so we just print it as is
    printf("[%s] path='%s' size=%lld dev=0x%lx ino=%llu\n", label, path, (long long)sb.st_size, (unsigned long)sb.st_dev, (unsigned long long)sb.st_ino);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    const char* path = argv[1];

    print_stat("Stage 1: Initial", path);

    printf("[Stage 2] Opening for O_RDWR (triggers COW)...\n");
    int fd = open(path, O_RDWR);
    if (fd < 0) { perror("open"); return 1; }

    const char* data = "New data that is definitely longer than original content 1234567890";
    if (write(fd, data, strlen(data)) < 0) { perror("write"); close(fd); return 1; }
    
    print_stat("Stage 3: While Open", path);

    close(fd);
    printf("[Stage 4] FD closed (triggers reingest)\n");

    print_stat("Stage 5: Post Close", path);

    printf("[Stage 6] Waiting for reingest tasks to complete...\n");
    sleep(2);

    print_stat("Stage 7: Final (updated manifest)", path);

    return 0;
}
EOF
gcc -o "$TEST_DIR/consistency_test" "$TEST_DIR/consistency_test.c"

# 5. Run test with Shim
export DYLD_INSERT_LIBRARIES="$SHIM_BIN"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_VFS_PREFIX="$(realpath "$VELO_PROJECT_ROOT")"
export VRIFT_DEBUG=1
export VRIFT_SOCKET_PATH="/tmp/vrift.sock"

# We must use absolute path for the test program to find the file
echo "[Test] Running consistency_test on $(realpath "$VELO_PROJECT_ROOT/test_file.txt")..."
"$TEST_DIR/consistency_test" "$(realpath "$VELO_PROJECT_ROOT/test_file.txt")"

echo "[Test] Result: $?"
echo "=== Success ==="
exit 0
