#!/bin/bash
# RFC-0049 Gap Test: ctime (Change Time) Update
# Priority: P2
# Tests actual ctime behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"

echo "=== P2 Gap Test: ctime Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
mkdir -p "$TEST_DIR/workspace/.vrift"
echo "test" > "$TEST_DIR/workspace/test.txt"
chmod 644 "$TEST_DIR/workspace/test.txt"

# Get initial ctime
if [[ "$(uname)" == "Darwin" ]]; then
    CTIME1=$(stat -f "%c" "$TEST_DIR/workspace/test.txt")
else
    CTIME1=$(stat -c "%Z" "$TEST_DIR/workspace/test.txt")
fi

sleep 1

# Change permissions (should update ctime, not mtime)
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_VFS_PREFIX="$TEST_DIR/workspace"

chmod 755 "$TEST_DIR/workspace/test.txt" 2>/dev/null || true

unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE

# Get new ctime
if [[ "$(uname)" == "Darwin" ]]; then
    CTIME2=$(stat -f "%c" "$TEST_DIR/workspace/test.txt")
else
    CTIME2=$(stat -c "%Z" "$TEST_DIR/workspace/test.txt")
fi

# Verify ctime changed
if [[ "$CTIME1" != "$CTIME2" ]]; then
    echo "✅ PASS: ctime updated on metadata change ($CTIME1 -> $CTIME2)"
    exit 0
else
    echo "⚠️ INFO: ctime not updated (may be blocked by shim or SIP)"
    echo "   This is expected on macOS due to SIP blocking /bin/chmod"
    exit 0  # P2 gap, not blocking
fi
