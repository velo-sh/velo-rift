#!/bin/bash
# ============================================================================
# Bug Reproduction: Shim RwLock Stress Hang
# ============================================================================
# This script stresses the shim's FD table by running multiple threads
# that open/close files simultaneously.
#
# If the shim uses RwLock, this is expected to hang or crash due to 
# recursive lock acquisition or lock-safety issues during bootstrap.

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
REPRO_SRC="$PROJECT_ROOT/tests/qa_v2/repro_rwlock_stress.c"
REPRO_BIN="$PROJECT_ROOT/tests/qa_v2/repro_rwlock_stress"

echo "----------------------------------------------------------------"
echo "🐞 Reproduction: Shim RwLock Stress Hang"
echo "----------------------------------------------------------------"

# 1. Compile Repro Tool
echo "🔨 Compiling repro tool..."
gcc -O3 "$REPRO_SRC" -o "$REPRO_BIN" -lpthread

# 2. Setup VFS Project
WORK_DIR="/tmp/vrift_repro_stress"
chflags -R nouchg "$WORK_DIR" 2>/dev/null || true
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/project/src"
echo "Target" > "$WORK_DIR/project/src/target.txt"

export VR_THE_SOURCE="$WORK_DIR/cas"

# Start daemon in background (needed for ingest since --direct was removed)
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
"$VRIFTD_BIN" start &>/dev/null &
DAEMON_PID=$!
sleep 1

"$VRIFT_BIN" init "$WORK_DIR/project" >/dev/null 2>&1
"$VRIFT_BIN" ingest "$WORK_DIR/project" --mode solid >/dev/null 2>&1

# Note: Daemon stays running for the stress test phase
# VFS_ENV below will connect to this active daemon.

VFS_ENV="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1 VRIFT_MANIFEST=$WORK_DIR/project/.vrift/manifest.lmdb VRIFT_VFS_PREFIX=$WORK_DIR/project VRIFT_LOG=info"

# 3. Run Stress Test
echo "🚀 Running stress test with 60s timeout..."
if command -v timeout &> /dev/null; then
    if timeout 60s env $VFS_ENV "$REPRO_BIN" "$WORK_DIR/project/src/target.txt" > "$WORK_DIR/stress.log" 2>&1; then
        echo "✅ Test Finished (No Hang detected)"
        cat "$WORK_DIR/stress.log" | head -n 5
    else
        EXIT_CODE=$?
        if [ $EXIT_CODE -eq 124 ]; then
            echo "🔥 BUG DETECTED: Multithreaded HANG (Timed out after 60s)"
            exit 1
        fi
        exit $EXIT_CODE
    fi
else
    env $VFS_ENV "$REPRO_BIN" "$WORK_DIR/project/src/target.txt" &
    PID=$!
    sleep 60
    if kill -0 $PID 2>/dev/null; then
        echo "🔥 BUG DETECTED: Multithreaded HANG (Still running after 60s)"
        kill -9 $PID 2>/dev/null || true
        exit 1
    else
        echo "✅ Test Finished (No Hang detected)"
        wait $PID || true
    fi
fi

echo "✅ Test Finished: No deadlock detected."
kill $DAEMON_PID 2>/dev/null || true
exit 0
