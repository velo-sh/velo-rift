#!/bin/bash
# Manual shim test script
set -x

cd /Users/antigravity/rust_source/velo-rift

TEST_DIR="/tmp/test_shim_quick"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/source"
echo -n "hello world" > "$TEST_DIR/source/testfile.txt"

# Ingest
VR_THE_SOURCE="$TEST_DIR/cas" ./target/debug/vrift ingest "$TEST_DIR/source" --prefix "vrift" 2>&1 | tail -3

# Start daemon
rm -f /tmp/vrift.sock
export VR_THE_SOURCE="$TEST_DIR/cas"
export VRIFT_MANIFEST_DIR="$TEST_DIR/source/.vrift/manifest.lmdb"
./target/debug/vriftd start > "$TEST_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!
echo "Daemon PID: $DAEMON_PID"
sleep 2

# Check socket
ls -la /tmp/vrift.sock 2>&1

# Simple C test
cat > "$TEST_DIR/test.c" << 'CEOF'
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>
int main() {
    printf("Opening /vrift/testfile.txt\n");
    fflush(stdout);
    int fd = open("/vrift/testfile.txt", O_RDONLY);
    printf("fd = %d\n", fd);
    if (fd >= 0) {
        char buf[100];
        int n = read(fd, buf, 99);
        buf[n > 0 ? n : 0] = 0;
        printf("Read: '%s'\n", buf);
        close(fd);
        return 0;
    }
    perror("open");
    return 1;
}
CEOF
gcc "$TEST_DIR/test.c" -o "$TEST_DIR/test"

# Run with shim
echo "=== Running with shim ==="
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="$PWD/target/debug/libvelo_shim.dylib"
export VRIFT_VFS_PREFIX="/vrift"
"$TEST_DIR/test"
RESULT=$?
echo "Exit code: $RESULT"

# Show daemon log
echo ""
echo "=== Daemon log ==="
cat "$TEST_DIR/daemon.log" | head -20

# Cleanup
kill $DAEMON_PID 2>/dev/null
rm -f /tmp/vrift.sock
rm -rf "$TEST_DIR"

exit $RESULT
