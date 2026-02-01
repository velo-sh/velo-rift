#!/bin/bash
# Test: Issue #4 - Manifest Desync (CLI writes LMDB, Daemon reads Bincode)
# Expected: FAIL (Daemon reports empty manifest after successful ingest)
# Fixed: SUCCESS (Daemon loads the same manifest CLI created)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Test: Manifest Desync (CLI vs Daemon) ==="
echo "Issue: CLI writes to LMDB, Daemon only loads Bincode, causing empty manifest in runtime."
echo ""

export VR_THE_SOURCE="/tmp/test_issue4_cas"
export VRIFT_MANIFEST="/tmp/test_issue4.manifest"
SOCKET_PATH="/tmp/vrift.sock"

# Setup (chflags first to handle leftover immutable files)
killall vriftd 2>/dev/null || true
chflags -R nouchg "$VR_THE_SOURCE" /tmp/test_issue4_dir 2>/dev/null || true
rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" "$SOCKET_PATH" /tmp/test_issue4_dir 2>/dev/null || true
mkdir -p "$VR_THE_SOURCE" /tmp/test_issue4_dir
echo "test content" > /tmp/test_issue4_dir/file.txt

# Step 1: Ingest
echo "[STEP 1] Ingesting directory..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" ingest /tmp/test_issue4_dir --output "$VRIFT_MANIFEST" --prefix /

# Step 2: Start Daemon
echo "[STEP 2] Starting daemon..."
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/daemon_issue4.log 2>&1 &
DAEMON_PID=$!
sleep 2

# Step 3: Check daemon's manifest status
echo "[STEP 3] Checking daemon manifest..."
DAEMON_LOG=$(cat /tmp/daemon_issue4.log)
if echo "$DAEMON_LOG" | grep -q "creating new"; then
    echo "[FAIL] Daemon did NOT load the manifest from CLI."
    echo "Daemon log:"
    cat /tmp/daemon_issue4.log
    EXIT_CODE=1
else
    echo "[PASS] Daemon loaded the manifest correctly."
    EXIT_CODE=0
fi

# Cleanup (use chflags to remove immutable flags first)
kill $DAEMON_PID 2>/dev/null || true
chflags -R nouchg "$VR_THE_SOURCE" 2>/dev/null || true
rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" "$SOCKET_PATH" /tmp/test_issue4_dir /tmp/daemon_issue4.log 2>/dev/null || true
exit $EXIT_CODE
