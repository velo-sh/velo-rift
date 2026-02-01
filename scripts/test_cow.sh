#!/bin/bash
# ============================================================================
# VRift Functional Test: CoW / Iron Law Protection
# ============================================================================

set -e

# Setup paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
VELO_BIN="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"

# Determine OS and preload variable
if [[ "$OSTYPE" == "darwin"* ]]; then
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
    OS_TYPE="Darwin"
else
    PRELOAD_VAR="LD_PRELOAD"
    OS_TYPE="Linux"
fi

# 1. Setup Test Workspace
TEST_DIR="${PROJECT_ROOT}/test_cow_work"
safe_rm "$TEST_DIR" 2>/dev/null || true
mkdir -p "$TEST_DIR"
PHYSICAL_ROOT="$TEST_DIR/source"
CAS_ROOT="$TEST_DIR/cas"
mkdir -p "$PHYSICAL_ROOT" "$CAS_ROOT"

echo "Testing CoW/Iron Law in $TEST_DIR"

# 2. Preparation
echo "Original Content" > "$PHYSICAL_ROOT/original.txt"

# 3. Ingest into VRift
echo ""
export VRIFT_CAS_ROOT="$(realpath "$CAS_ROOT")"
"$VELO_BIN" ingest "$PHYSICAL_ROOT" --output "$TEST_DIR/manifest.lmdb"

CAS_FILE=$(find "$CAS_ROOT" -type f | grep -v "\.lock" | head -n 1)
echo "CAS File: $CAS_FILE"

# 4. Start vriftd
pkill vriftd 2>/dev/null || true
sleep 1
export VRIFT_MANIFEST="$TEST_DIR"
"$VRIFTD_BIN" > "$TEST_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!

cleanup() {
    echo "Cleaning up..."
    kill $DAEMON_PID 2>/dev/null || true
    rm -rf "$TEST_DIR" 2>/dev/null || true
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
    echo "ERROR: Daemon failed to start. Log content:"
    cat "$TEST_DIR/daemon.log"
    exit 1
fi

# 5. Scenario A: Direct modification (WITHOUT SHIM)
echo ""
echo "Step 1: Modifying WITHOUT shim (Should fail or be blocked if possible)..."
# Note: On Linux as root, even chmod -w doesn't block writes.
chmod -w "$PHYSICAL_ROOT/original.txt" 2>/dev/null || true
if ! echo "corrupted" > "$PHYSICAL_ROOT/original.txt" 2>/dev/null; then
    echo "✓ CAS protected! (Direct write blocked)"
else
    # Check if CAS was actually changed
    CORRUPT_CONTENT=$(cat "$CAS_FILE")
    if [ "$CORRUPT_CONTENT" == "corrupted" ]; then
        echo "⚠️  CAS IS CORRUPTED (Iron Law not enforced at FS level - Expected for root/unprivileged Docker)"
        # Restore CAS for the real test (Step 2)
        echo "Original Content" > "$CAS_FILE"
    else
        echo "✓ CAS is safe."
    fi
fi

# 6. Scenario B: Modification (WITH SHIM)
echo ""
echo "Step 2: Testing WITH shim (Simulating CoW)..."

# Find the shim
if [[ "$OS_TYPE" == "Darwin" ]]; then
    SHIM_BIN=$(find "$PROJECT_ROOT/target/release" -name "libvrift_shim.dylib" | head -n 1)
else
    SHIM_BIN=$(find "$PROJECT_ROOT/target/release" -name "libvrift_shim.so" | head -n 1)
fi

if [[ -z "$SHIM_BIN" ]]; then
    echo "ERROR: libvrift_shim not found."
    exit 1
fi

echo "Using shim: $SHIM_BIN"

export VRIFT_VFS_PREFIX="$PHYSICAL_ROOT"
export VRIFT_DEBUG=1
export "$PRELOAD_VAR"="$(realpath "$SHIM_BIN")"
if [[ "$OS_TYPE" == "Darwin" ]]; then
    export DYLD_FORCE_FLAT_NAMESPACE=1
fi

TEST_PATH="$PHYSICAL_ROOT/original.txt"

# This SHOULD trigger CoW, breaking link, and allowing write without corrupting CAS
echo "Running writer with shim on $TEST_PATH..."
# We MUST use a separate process because the shell redirection is not shimmed in the current shell
sh -c "echo 'new content' > '$TEST_PATH'"

# Clear preload for verification
unset "$PRELOAD_VAR"
unset DYLD_FORCE_FLAT_NAMESPACE

NEW_CAS_CONTENT=$(cat "$CAS_FILE")
if [ "$NEW_CAS_CONTENT" == "Original Content" ]; then
    echo "✅ Success: CoW protected the CAS!"
else
    echo "❌ Failure: CAS was corrupted even with shim."
    echo "Actual CAS content: $NEW_CAS_CONTENT"
    exit 1
fi

# 7. Check if re-ingested (by-product of CoW in some impls)
NEW_COUNT=$(find "$CAS_ROOT" -type f | grep -v "\.lock" | wc -l)
echo "Blobs in CAS: $NEW_COUNT"

echo ""
echo "=== Test Passed ==="
exit 0
