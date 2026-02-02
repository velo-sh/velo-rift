#!/bin/bash
# RFC-0049 Gap Test: flock() Semantic Isolation
#
# This is a P0 gap that WILL break ccache and parallel builds
#
# Problem: flock() on temp file ≠ logical file lock
# Impact: Two processes both think they have exclusive lock

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P0 Gap Test: flock() Semantic Isolation ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[1] Compiling test helper..."
cat > flock_test.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/file.h>
#include <sys/time.h>
#include <errno.h>

long current_ms() {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec * 1000 + tv.tv_usec / 1000;
}

int main(int argc, char *argv[]) {
    if (argc < 4) {
        fprintf(stderr, "Usage: %s <file> <op> <sleep_ms>\n", argv[0]);
        return 1;
    }

    const char *path = argv[1];
    int op = atoi(argv[2]); // 2=EX, 1=SH, 8=UN
    int sleep_ms = atoi(argv[3]);

    int fd = open(path, O_RDWR | O_CREAT, 0666);
    if (fd < 0) {
        perror("open");
        return 1;
    }

    // printf("PID %d: Acquiring lock...\n", getpid());
    long t0 = current_ms();
    if (flock(fd, op) != 0) {
        perror("flock");
        return 1;
    }
    long t1 = current_ms();
    printf("PID %d: Acquired lock in %ld ms\n", getpid(), t1 - t0);

    if (sleep_ms > 0) {
        usleep(sleep_ms * 1000);
    }

    flock(fd, LOCK_UN);
    close(fd);
    return 0;
}
EOF

gcc -o flock_test flock_test.c

echo ""
echo "[2] Starting Daemon..."
export VELO_PROJECT_ROOT="${SCRIPT_DIR}/test_flock_root"
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
rm -rf "$VELO_PROJECT_ROOT/.vrift/socket"
DAEMON_BIN="${PROJECT_ROOT}/target/debug/vriftd"
# Start Daemon (isolate from shim injection)
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    unset LD_PRELOAD
    export RUST_LOG=debug
    $DAEMON_BIN start > "$VELO_PROJECT_ROOT/daemon.log" 2>&1 &
    echo $! > "$VELO_PROJECT_ROOT/daemon.pid"
)
DAEMON_PID=$(cat "$VELO_PROJECT_ROOT/daemon.pid")
sleep 2

# Register workspace logic (mimic CLI)
# For this test, we might need a simpler way or assume daemon auto-registers?
# Daemon requires explicit registration usually.
# But vriftd auto-scans? No.
# Use ipc-cli or manually register?
# For now, let's create a minimal test where we assume correct setup or update daemon later.
# Wait, VFS logic requires workspace to be registered.

# Let's bypass registration for raw test or register it.
# Simple way: send register IPC.
# Or use python script to register.
# Or: Daemon checks `load_registered_workspaces`.
# We can write to ~/.vrift/registry/manifests.json

mkdir -p "$HOME/.vrift/registry"
echo "{\"manifests\": {\"test\": {\"project_root\": \"$VELO_PROJECT_ROOT\"}}}" > "$HOME/.vrift/registry/manifests.json"
kill $DAEMON_PID || true
sleep 1
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    unset LD_PRELOAD
    export RUST_LOG=debug
    $DAEMON_BIN start >> "$VELO_PROJECT_ROOT/daemon.log" 2>&1 &
    echo $! > "$VELO_PROJECT_ROOT/daemon.pid"
)
DAEMON_PID=$(cat "$VELO_PROJECT_ROOT/daemon.pid")
# Wait for socket
for i in {1..20}; do
    if [[ -S "${VELO_PROJECT_ROOT}/.vrift/socket" ]]; then
        echo "Daemon socket found after $i attempts."
        break
    fi
    sleep 0.5
done
sleep 1

echo ""
echo "[3] Running functional test..."
if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi
export VRIFT_SOCKET_PATH="${VELO_PROJECT_ROOT}/.vrift/socket"

# Create file in VFS
TEST_FILE="$VELO_PROJECT_ROOT/lock.txt"
touch "$TEST_FILE"

# Process A: Hold lock for 1000ms
./flock_test "$TEST_FILE" 2 1000 &
PID_A=$!

sleep 0.2

# Process B: Try to acquire. Should block ~800ms
t_start=$(date +%s%3N)
# Mac date doesn't support %3N well, use python or helper
# Better: rely on helper output.
./flock_test "$TEST_FILE" 2 0 > output_b.txt
PID_B=$!

wait $PID_A
wait $PID_B

kill $DAEMON_PID



# Analyze Output
cat output_b.txt
WAIT_MS=$(grep "Acquired lock in" output_b.txt | awk '{print $6}')

if [[ -z "$WAIT_MS" ]]; then
    echo "❌ ERROR: Could not parse wait time from output_b.txt"
    exit 1
fi

echo "Process B waited: ${WAIT_MS} ms"

if (( WAIT_MS > 500 )); then
    echo "✅ PASS: Flock blocking behavior confirmed (> 500ms wait)"
    rm flock_test flock_test.c output_b.txt
    exit 0
else
    echo "❌ FAIL: Flock acquired immediately (Wait: ${WAIT_MS} ms). Isolation failed."
    echo "=== Daemon Logs ==="
    cat "$VELO_PROJECT_ROOT/daemon.log"
    rm flock_test flock_test.c output_b.txt
    exit 1
fi
