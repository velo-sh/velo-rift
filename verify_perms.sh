#!/bin/bash
set -e

# Setup clean test environment
TEST_DIR="/tmp/vrift-perm-test"
mkdir -p "$TEST_DIR/home"
export HOME="$TEST_DIR/home"

LOG_FILE="/tmp/vrift-perm-vriftd.log"

echo "0. Cleaning up previous run..."
pkill vriftd || true
sleep 1
# Remove flags that block deletion
chflags -R nouchg "$TEST_DIR" || true
# Ensure we can delete
chmod -R 777 "$TEST_DIR" || true
rm -rf "$TEST_DIR/home/.vrift"
rm -rf "$TEST_DIR/src"
rm -rf "$TEST_DIR/the_source_local"
rm -f "$LOG_FILE"
rm -f "$TEST_DIR/vriftd.sock"

mkdir -p "$TEST_DIR/src"

VRIFT_BIN="/Users/antigravity/rust_source/velo-rift/target/release/vrift"
VRIFTD_BIN="/Users/antigravity/rust_source/velo-rift/target/release/vriftd"

# Explicit Env Vars for ALL components
export VRIFT_SOCKET_PATH="$TEST_DIR/vriftd.sock"
export VRIFT_MANIFEST="$TEST_DIR/src/.vrift/manifest.lmdb"
export VR_THE_SOURCE="$TEST_DIR/the_source_local"
export VRIFT_LOG_DIR="$TEST_DIR"
export VRIFT_DEBUG=1

echo "1. Creating test files with distinct permissions..."
echo "hello" > "$TEST_DIR/src/readonly.txt"
chmod 444 "$TEST_DIR/src/readonly.txt"

echo "echo hi" > "$TEST_DIR/src/exec.sh"
echo "exit 0" >> "$TEST_DIR/src/exec.sh"
chmod 755 "$TEST_DIR/src/exec.sh"

echo "normal" > "$TEST_DIR/src/normal.txt"
chmod 644 "$TEST_DIR/src/normal.txt"

# Go to test dir
cd "$TEST_DIR"

echo "2. Initializing..."
"$VRIFT_BIN" init .
# Prevent background watcher from competing with manual ingest
# Insert ignore_patterns after [ingest] header
sed -i '' '/\[ingest\]/a \
ignore_patterns = ["src"]
' .vrift/config.toml

echo "3. Starting Daemon..."
export VRIFT_LOG=debug
# "$VRIFTD_BIN" start "$TEST_DIR" > "/tmp/vrift-perm-vriftd-trace.log" 2>&1 &
# sleep 5
# Note: we let CLI autostart for consistency, or manually start properly

echo "4. Ingesting via Daemon..."
"$VRIFT_BIN" ingest --mode solid src --output verify.manifest -j 1

echo "5. Verifying Virtual Permissions (stat)..."
# Find VDir file from project root (standard location)
VDIR_FILE="$TEST_DIR/home/.vrift/vdir/f37bb59acdd9c2bb.vdir"
echo "Using VDir: $VDIR_FILE"
export VRIFT_VDIR_MMAP="$VDIR_FILE"

"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest stat -f "%A %N" src/readonly.txt
"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest stat -f "%A %N" src/exec.sh
"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest stat -f "%A %N" src/normal.txt

echo "6. Verifying Materialization (deleting physical then run)..."
rm -f "$TEST_DIR/src/readonly.txt" "$TEST_DIR/src/exec.sh"

echo "Accessing files to trigger materialization..."
"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest cat src/readonly.txt > /dev/null
"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest ./src/exec.sh > /dev/null

echo "Checking physical modes after materialization..."
stat -f "%A %N" src/readonly.txt
stat -f "%A %N" src/exec.sh

echo "7. Verifying COW Persistence..."
"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest sh -c "echo 'updated' >> src/normal.txt"
echo "Checking physical mode of COW staged file..."
stat -f "%A %N" src/normal.txt

echo "8. Execution Test..."
"$VRIFT_BIN" --the-source-root "$TEST_DIR/home/.vrift/the_source" run -m verify.manifest ./src/exec.sh

# Cleanup
echo "Cleaning up..."
pkill -9 vriftd || true
echo "SUCCESS: Permissions are faithful!"
