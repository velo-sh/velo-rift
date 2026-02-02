#!/bin/bash
# repro_shim_init_race.sh
# Solidifies the bug where early VFS readiness checks cause the first call to passthrough.

set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VELO_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
# OS Detection
if [ "$(uname -s)" == "Darwin" ]; then
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
else
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.so"
    PRELOAD_VAR="LD_PRELOAD"
fi

TEST_DIR=$(mktemp -d)
# trap "rm -rf '$TEST_DIR'" EXIT  <-- Disabled for debugging
echo "DEBUG: TEST_DIR=$TEST_DIR"
mkdir -p "$TEST_DIR/source"

# 1. Ingest
export VRIFT_CAS_ROOT="$TEST_DIR/cas"
# Ingest with prefix / and ensure we are ingesting the right thing
echo "secret content" > "$TEST_DIR/source/file.txt"
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix / > "$TEST_DIR/ingest.log" 2>&1

# 2. Start daemon
export VRIFT_MANIFEST="$TEST_DIR/source/.vrift/manifest.lmdb"
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
sleep 2

# 3. Reproductive Step: Try to cat the virtual file
# Avoid SIP and Signature issues by compiling a tiny cat
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

echo "--- Proof Analysis ---"
# We expect success now because the race/deadlock is fixed.
# If it fails with "No such file", it means interception is NOT working.
# If it fails with "No such file", it means interception is NOT working.
if env "$PRELOAD_VAR"="$SHIM_LIB" VRIFT_VFS_PREFIX="/vrift" "$TEST_DIR/cat" /vrift/file.txt | grep -q "secret content"; then
    echo "SUCCESS: VFS Interception worked for early call!"
else
    echo "FAILURE: VFS Interception failed."
    # Dump logs for diagnostics
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

# Cleanup
pkill vriftd || true
if [ "$(uname -s)" == "Darwin" ]; then
    chflags -R nouchg "$TEST_DIR" || true
fi
rm -rf "$TEST_DIR"
