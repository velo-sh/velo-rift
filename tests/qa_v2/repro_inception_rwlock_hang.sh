#!/bin/bash
# ============================================================================
# Bug Reproduction: Shim Bootstrap Deadlock (Pattern 2648)
# ============================================================================
# This script reproduces the hang observed when using RwLock in the inception's 
# global state (io.rs) during process bootstrap on macOS ARM64.
#
# Finding: RwLock in Rust's stdlib triggers TLS/Pthread features that are not
# safe during dyld's interpose initialization.
#
# Required: Revert crates/vrift-inception/src/syscalls/io.rs to use RwLock.

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
INCEPTION_LIB="$PROJECT_ROOT/target/release/libvrift_inception.dylib"
MINI_READ="$PROJECT_ROOT/tests/qa_v2/mini_read"

# Setup work dir (ignore errors from VFS-protected files in previous run)
WORK_DIR="/tmp/vrift_repro_hang"
rm -rf "$WORK_DIR" 2>/dev/null || true
mkdir -p "$WORK_DIR/project/src"
echo "Repro Content" > "$WORK_DIR/project/src/hello.txt"

echo "----------------------------------------------------------------"
echo "🐞 Reproduction: Shim Bootstrap Hang (RwLock)"
echo "----------------------------------------------------------------"

# 1. Start daemon and Ingest
echo "⚡ Preparing VFS Project..."
export VR_THE_SOURCE="$WORK_DIR/cas"

# Start daemon in background (needed for ingest since --direct was removed)
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
$VRIFTD_BIN start &>/dev/null &
DAEMON_PID=$!
sleep 1

$VRIFT_BIN init "$WORK_DIR/project" >/dev/null 2>&1
$VRIFT_BIN ingest "$WORK_DIR/project" --mode solid >/dev/null 2>&1

# Note: Daemon stays running for the test phase
# VFS_ENV below will connect to this active daemon.

VFS_ENV="DYLD_INSERT_LIBRARIES=$INCEPTION_LIB DYLD_FORCE_FLAT_NAMESPACE=1 VRIFT_MANIFEST=$WORK_DIR/project/.vrift/manifest.lmdb VRIFT_VFS_PREFIX=$WORK_DIR/project"

# 2. Trigger Hang
echo "🚀 Running test with 5s timeout (Expected to HANG if bug exists)..."

# We use 'timeout' (if available) or a background process kill
if command -v timeout &> /dev/null; then
    if timeout 5s env $VFS_ENV "$MINI_READ" "$WORK_DIR/project/src/hello.txt" > "$WORK_DIR/output.log" 2>&1; then
        echo "✅ Test Finished (No Hang detected - code might be patched)"
    else
        EXIT_CODE=$?
        if [ $EXIT_CODE -eq 124 ]; then
            echo "🔥 BUG DETECTED: Process HANGED (Timed out after 5s)"
            echo "   This confirms the RwLock deadlock in Shim bootstrap."
            exit 1
        else
            echo "❌ Unexpected failure (Exit Code: $EXIT_CODE)"
            cat "$WORK_DIR/output.log"
            exit 1
        fi
    fi
else
    # Simple background kill if no timeout command
    env $VFS_ENV "$MINI_READ" "$WORK_DIR/project/src/hello.txt" > "$WORK_DIR/output.log" 2>&1 &
    PID=$!
    sleep 5
    if kill -0 $PID 2>/dev/null; then
        echo "🔥 BUG DETECTED: Process HANGED (Still running after 5s)"
        kill -9 $PID 2>/dev/null || true
        exit 1
    else
        echo "✅ Test Finished (No Hang detected)"
        wait $PID || true
    fi
fi

echo "✅ Test Finished: No bootstrap deadlock detected."
kill $DAEMON_PID 2>/dev/null || true

# Cleanup (ignore errors from VFS-protected files)
rm -rf "$WORK_DIR" 2>/dev/null || true
exit 0
