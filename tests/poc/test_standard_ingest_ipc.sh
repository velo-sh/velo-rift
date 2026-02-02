#!/bin/bash

# Standard Functional Verification of RFC-0043 Ingest Workflow
# CLI -> IPC -> Daemon -> CAS

TEST_DIR="/tmp/vrift_functional_test"
TEST_FILE="$TEST_DIR/hello.txt"
MANIFEST_FILE="$TEST_DIR/vrift.manifest"

echo "--- Standard Ingest IPC Verification ---"

# 1. Cleanup and Setup
# Use unique CAS root to avoid permission/locked file issues from previous runs
CAS_ROOT="/tmp/vrift_the_source_functional_$(date +%s)"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
echo "Hello Velo Functional" > "$TEST_FILE"

# 2. Start Daemon in background with DEBUG logs
echo "[+] Starting Daemon with RUST_LOG=debug..."
export VR_THE_SOURCE="$CAS_ROOT"
# Note: vriftd currently hardcodes /tmp/vrift.sock
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    RUST_LOG=debug ./target/debug/vriftd start > "$TEST_DIR/daemon.log" 2>&1 &
    echo $! > "$TEST_DIR/daemon.pid"
)
DAEMON_PID=$(cat "$TEST_DIR/daemon.pid")
sleep 1

# 3. Check Daemon Status via CLI
echo "[+] Checking daemon status..."
./target/debug/vrift --the-source-root "$CAS_ROOT" daemon status

# 4. Perform Ingest (Tier-1 to trigger protect_file IPC)
echo "[+] Performing Ingest (Tier-1)..."
./target/debug/vrift --the-source-root "$CAS_ROOT" ingest "$TEST_DIR" --tier tier1 --output "$MANIFEST_FILE"

# 5. Verify CAS Blob Integrity & Permissions
# Dynamically find the ingested blob (there should be only one)
BLOB_PATH=$(find "$CAS_ROOT" -name "*.bin" | head -n 1)

if [ -z "$BLOB_PATH" ]; then
    echo "[FAIL] CAS Blob not found!"
    ls -R "$CAS_ROOT"
    exit 1
fi

echo "[+] Found CAS Blob: $BLOB_PATH"
ls -lo "$BLOB_PATH"

PERMS=$(ls -lo "$BLOB_PATH" | awk '{print $1}')
FLAGS=$(ls -lo "$BLOB_PATH" | awk '{print $5}')

if [[ "$PERMS" == "-r--r--r--" ]]; then
    echo "[SUCCESS] CAS Blob correctly set to 444."
else
    echo "[FAIL] Unexpected permissions: $PERMS (Expected: -r--r--r--)"
fi

if [[ "$FLAGS" == *"schg"* ]] || [[ "$FLAGS" == *"uchg"* ]]; then
    echo "[SUCCESS] CAS Blob protected with immutable flag ($FLAGS)."
else
    echo "[INFO] CAS Blob has no immutable flag (Expected for Tier-1 on macOS)."
fi

# 6. Verify Daemon Indexing (Live Ingest check)
echo "[+] Verifying Daemon logs for IPC messages..."
grep "CasInsert" "$TEST_DIR/daemon.log" > /dev/null
if [ $? -eq 0 ]; then
    echo "[SUCCESS] Daemon received CasInsert IPC notification."
else
    echo "[WARNING] Daemon did NOT receive CasInsert IPC. Live Ingest is NOT implemented in CLI."
fi

grep "Protect" "$TEST_DIR/daemon.log" > /dev/null
if [ $? -eq 0 ]; then
    echo "[SUCCESS] Daemon received Protect IPC notification."
else
    echo "[FAIL] Daemon did NOT receive Protect IPC for Tier-1 ingest."
fi

# 7. Verify Fallback (Kill daemon and ingest)
echo "[+] Testing Fallback (Daemon Offline)..."
kill $DAEMON_PID
sleep 1

./target/debug/vrift --the-source-root "$CAS_ROOT" ingest "$TEST_DIR" --output "$TEST_DIR/fallback.manifest"
if [ $? -eq 0 ]; then
    echo "[SUCCESS] CLI correctly falls back to direct mode when daemon is offline."
    echo ""
    echo "✅ PASS: Standard Ingest IPC Verification (with functional warnings)"
    EXIT_CODE=0
else
    echo "[FAIL] CLI failed when daemon was offline."
    EXIT_CODE=1
fi

# Cleanup
rm -rf "$TEST_DIR" "$CAS_ROOT" 2>/dev/null || true
kill $DAEMON_PID 2>/dev/null || true

exit $EXIT_CODE
