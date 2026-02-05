#!/bin/bash
# E2E Test: Live Ingest via vrift-vdird daemon
#
# Tests the full ingest pipeline:
# 1. Start daemon
# 2. Create/modify files
# 3. Verify CAS content and manifest entries

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TEST_DIR="$PROJECT_ROOT/target/test_live_ingest_$$"
DAEMON_PID=""
DAEMON_LOG="$TEST_DIR/daemon.log"

cleanup() {
    echo "Cleaning up..."
    [ -n "$DAEMON_PID" ] && kill $DAEMON_PID 2>/dev/null || true
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Setup test directory
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Initialize .vrift directory
mkdir -p .vrift

echo "=== E2E Live Ingest Test ==="
echo "Test directory: $TEST_DIR"

# Build the daemon
echo "Building vrift-vdird..."
(cd "$PROJECT_ROOT" && cargo build -p vrift-vdird --release 2>/dev/null) || {
    echo -e "${RED}Failed to build vrift-vdird${NC}"
    exit 1
}

VDIRD="$PROJECT_ROOT/target/release/vrift-vdird"
if [ ! -f "$VDIRD" ]; then
    VDIRD="$PROJECT_ROOT/target/debug/vrift-vdird"
fi

# Start daemon in background
echo "Starting vrift-vdird daemon..."
$VDIRD "$TEST_DIR" > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
sleep 2

if ! kill -0 $DAEMON_PID 2>/dev/null; then
    echo -e "${RED}Daemon failed to start${NC}"
    cat "$DAEMON_LOG"
    exit 1
fi
echo "Daemon started with PID: $DAEMON_PID"

# Wait for FSWatch to initialize
sleep 2

# Test 1: Create a new file
echo ""
echo "--- Test 1: File Creation ---"
echo "Hello, Live Ingest!" > hello.txt

# Retry logic for file creation detection (up to 5 attempts)
MAX_RETRIES=5
for i in $(seq 1 $MAX_RETRIES); do
    sleep 1
    if grep -q "Ingest: file stored to CAS" "$DAEMON_LOG"; then
        echo -e "${GREEN}✓ File creation detected and stored to CAS (attempt $i)${NC}"
        break
    fi
    if [ $i -eq $MAX_RETRIES ]; then
        echo -e "${RED}✗ File creation not detected after $MAX_RETRIES attempts${NC}"
        cat "$DAEMON_LOG"
        exit 1
    fi
    echo "  Waiting for ingest... (attempt $i/$MAX_RETRIES)"
done

# Test 2: Modify a file
echo ""
echo "--- Test 2: File Modification ---"
echo "Modified content" >> hello.txt
sleep 1

# Should see another ingest event
INGEST_COUNT=$(grep -c "Ingest: file stored to CAS" "$DAEMON_LOG" || echo 0)
if [ "$INGEST_COUNT" -ge 2 ]; then
    echo -e "${GREEN}✓ File modification detected (total ingest events: $INGEST_COUNT)${NC}"
else
    echo -e "${RED}✗ File modification not detected${NC}"
fi

# Test 3: Create a directory
echo ""
echo "--- Test 3: Directory Creation ---"
mkdir -p subdir
sleep 1

if grep -q "Ingest: directory registered" "$DAEMON_LOG"; then
    echo -e "${GREEN}✓ Directory creation detected${NC}"
else
    echo -e "${RED}✗ Directory creation not detected${NC}"
fi

# Test 4: Create a symlink
echo ""
echo "--- Test 4: Symlink Creation ---"
ln -s hello.txt link_to_hello
sleep 1

if grep -q "Ingest: symlink stored to CAS" "$DAEMON_LOG"; then
    echo -e "${GREEN}✓ Symlink creation detected and target stored${NC}"
else
    echo -e "${RED}✗ Symlink creation not detected${NC}"
fi

# Test 5: Verify CAS storage
echo ""
echo "--- Test 5: CAS Verification ---"
CAS_ROOT="${HOME}/.vrift/the_source"
if [ -d "$CAS_ROOT" ]; then
    BLOB_COUNT=$(find "$CAS_ROOT" -type f | wc -l | tr -d ' ')
    echo "CAS contains $BLOB_COUNT blobs"
    if [ "$BLOB_COUNT" -gt 0 ]; then
        echo -e "${GREEN}✓ CAS has stored blobs${NC}"
    else
        echo -e "${RED}✗ CAS is empty${NC}"
    fi
else
    echo -e "${RED}✗ CAS directory not found${NC}"
fi

# Test 6: Verify tier classification (from log)
echo ""
echo "--- Test 6: Tier Classification ---"
if grep -q "tier=" "$DAEMON_LOG"; then
    echo -e "${GREEN}✓ Tier classification logged${NC}"
    grep "tier=" "$DAEMON_LOG" | head -2
else
    echo -e "${RED}✗ Tier classification not logged${NC}"
fi

# Summary
echo ""
echo "=== Test Summary ==="
grep "Ingest:" "$DAEMON_LOG" | tail -10

echo ""
echo -e "${GREEN}E2E Live Ingest Test Completed${NC}"
