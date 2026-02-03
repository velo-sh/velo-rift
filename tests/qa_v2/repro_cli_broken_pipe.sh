#!/bin/bash
# ============================================================================
# Bug Reproduction: CLI Broken Pipe Panic
# ============================================================================
# Reproduction for the panic: 
# "thread 'main' panicked at ... failed printing to stdout: Broken pipe"
#
# This happens because Rust's println! panics on EPIPE if the receiver
# (like grep -q) closes the pipe early.

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"

# Setup work dir
WORK_DIR="/tmp/vrift_repro_pipe"
chflags -R nouchg "$WORK_DIR" 2>/dev/null || true
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/project"

echo "----------------------------------------------------------------"
echo "🐞 Reproduction: CLI Broken Pipe Panic"
echo "----------------------------------------------------------------"

# 1. Start Daemon
pkill vriftd 2>/dev/null || true
sleep 1
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb"
"$PROJECT_ROOT/target/release/vriftd" start > "$WORK_DIR/daemon.log" 2>&1 &
DAEMON_PID=$!

cleanup() {
    kill $DAEMON_PID 2>/dev/null || true
    chflags -R nouchg "$WORK_DIR" 2>/dev/null || true
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# Wait for daemon
sleep 2

# 2. Initialize
cd "$WORK_DIR/project"
"$VRIFT_BIN" init . >/dev/null 2>&1

echo "🚀 Triggering pipe closure (vrift status | grep -q)..."

# Run with timeout to prevent test hang
# We expect panic on stderr.
if command -v timeout &> /dev/null; then
    timeout 5s bash -c "$VRIFT_BIN status 2> $WORK_DIR/stderr.log | grep -q 'Velo Rift Status'" || true
else
     # Fallback for systems without timeout command (macOS default often doesn't have it)
     ( "$VRIFT_BIN" status 2> "$WORK_DIR/stderr.log" | grep -q "Velo Rift Status" ) &
     PID=$!
     sleep 5
     kill $PID 2>/dev/null || true
fi

if grep -q "panicked" "$WORK_DIR/stderr.log"; then
    echo "🔥 BUG DETECTED: CLI Panicked with Broken Pipe!"
    cat "$WORK_DIR/stderr.log"
    exit 1
fi

if grep -q "Broken pipe" "$WORK_DIR/stderr.log"; then
     echo "🔥 BUG DETECTED: CLI reported Broken pipe!"
     cat "$WORK_DIR/stderr.log"
     exit 1
fi

echo "✅ Test Finished: No panic detected."
exit 0
