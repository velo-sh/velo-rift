#!/bin/bash
# Test: Dirty Consistency Verification (COW Triggered)
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== Test: Dirty Consistency (COW Metadata) ==="

# Setup VFS project by ingesting a real file
mkdir -p "$VELO_PROJECT_ROOT"
echo "original content" > "$VELO_PROJECT_ROOT/test_file.txt"

# Ingest into CAS and create LMDB manifest
export VR_THE_SOURCE="$TEST_DIR/cas"
mkdir -p "$TEST_DIR/cas"

# Kill pre-existing daemon so test env is used
pkill -9 vriftd 2>/dev/null || true
sleep 1

# Use release binary with debug fallback
VRIFT_BIN="${PROJECT_ROOT}/target/release/vrift"
if [ ! -f "$VRIFT_BIN" ]; then
    VRIFT_BIN="${PROJECT_ROOT}/target/debug/vrift"
fi
"$VRIFT_BIN" ingest "$VELO_PROJECT_ROOT" --prefix "" 2>/dev/null || true

# Compile test program
cat > "$TEST_DIR/consistency_test.c" << 'EOF'
#include <stdio.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

void print_stat(const char* label, const char* path) {
    struct stat sb;
    if (stat(path, &sb) != 0) {
        perror("stat");
        return;
    }
    printf("[%s] path='%s' size=%lld dev=0x%llx\n", label, path, (long long)sb.st_size, (unsigned long long)sb.st_dev);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 2;
    const char* path = argv[1];

    // 1. Initial stat (should be RIFT manifest)
    print_stat("Initial", path);

    // 2. Open for write (should trigger COW)
    printf("[Open] Opening for O_RDWR...\n");
    int fd = open(path, O_RDWR);
    if (fd < 0) { perror("open"); return 1; }

    // 3. Write data
    const char* data = "New data that is definitely longer than sixteen bytes";
    printf("[Write] Writing %lu bytes...\n", (unsigned long)strlen(data));
    if (write(fd, data, strlen(data)) < 0) { perror("write"); close(fd); return 1; }
    
    // 4. Stat while open (should be live from temp file, dev=0x52494654)
    print_stat("While Open", path);

    // 5. Close
    close(fd);
    printf("[Close] FD closed\n");

    // 6. Stat immediately after close (should still be live from staging, as dirty is delayed)
    print_stat("Post Close", path);

    return 0;
}
EOF
gcc -o "$TEST_DIR/consistency_test" "$TEST_DIR/consistency_test.c"

# Environment Setup
export VR_THE_SOURCE="$TEST_DIR/cas"
export VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock"
export VRIFT_MANIFEST="$TEST_DIR/workspace/.vrift/manifest.lmdb"

# 1. Ingest
echo "Ingesting project..."
"$VRIFT_BIN" ingest "$TEST_DIR/workspace" --prefix "" > "$TEST_DIR/ingest.log" 2>&1
cat "$TEST_DIR/ingest.log"

# 2. Start Daemon (if not already started)
"$VRIFTD_BIN" start >> "$TEST_DIR/daemon.log" 2>&1 &
VRIFT_PID=$!
sleep 2

# Use release inception layer with debug fallback
INCEPTION_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
if [ ! -f "$INCEPTION_LIB" ]; then
    INCEPTION_LIB="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
fi

DYLD_INSERT_LIBRARIES="$INCEPTION_LIB" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
VRIFT_SOCKET_PATH="$TEST_DIR/vrift.sock" \
VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT" \
VRIFT_DEBUG=1 \
"$TEST_DIR/consistency_test" "$VELO_PROJECT_ROOT/test_file.txt"

RET=$?

# Cleanup properly even with immutable files
if [ -d "$TEST_DIR" ]; then
    # Clear immutable flags on CAS blobs before rm
    find "$TEST_DIR" -type f -exec chflags nouchg {} + 2>/dev/null || true
    rm -rf "$TEST_DIR"
fi
exit $RET
