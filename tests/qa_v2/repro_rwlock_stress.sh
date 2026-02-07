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
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
REPRO_SRC="$PROJECT_ROOT/tests/qa_v2/repro_rwlock_stress.c"
REPRO_BIN="/tmp/repro_rwlock_stress_$$"

echo "----------------------------------------------------------------"
echo "🐞 Reproduction: Shim RwLock Stress Hang"
echo "----------------------------------------------------------------"

WORK_DIR="/tmp/vrift_repro_stress_$$"
export VR_THE_SOURCE="$WORK_DIR/cas"
export VRIFT_SOCKET_PATH="$WORK_DIR/vrift.sock"
DAEMON_PID=""

cleanup() {
    [ -n "$DAEMON_PID" ] && kill -9 "$DAEMON_PID" 2>/dev/null || true
    rm -f "$VRIFT_SOCKET_PATH"
    rm -f "$REPRO_BIN"
    chflags -R nouchg "$WORK_DIR" 2>/dev/null || true
    rm -rf "$WORK_DIR" 2>/dev/null || true
}
trap cleanup EXIT

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/project/src" "$VR_THE_SOURCE"
echo "Target" > "$WORK_DIR/project/src/target.txt"

# 1. Compile Repro Tool
echo "🔨 Compiling repro tool..."
gcc -O3 "$REPRO_SRC" -o "$REPRO_BIN" -lpthread

# 2. Setup VFS Project
echo "📦 Setting up VFS project..."
cd "$WORK_DIR/project"
"$VRIFT_BIN" init . >/dev/null 2>&1

# Start daemon (needed for ingest since --direct was removed)
"$VRIFTD_BIN" start </dev/null > "$WORK_DIR/vriftd.log" 2>&1 &
DAEMON_PID=$!

# Wait for socket
waited=0
while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
    sleep 0.5
    waited=$((waited + 1))
done

"$VRIFT_BIN" ingest "$WORK_DIR/project" --mode solid --output "$WORK_DIR/project/.vrift/manifest.lmdb" >/dev/null 2>&1

# 3. Run Stress Test with macOS-compatible timeout
echo "🚀 Running stress test with 60s timeout..."
STRESS_PID=""
env DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 \
    VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
    VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb" \
    VRIFT_VFS_PREFIX="$WORK_DIR/project" VRIFT_LOG=info \
    "$REPRO_BIN" "$WORK_DIR/project/src/target.txt" > "$WORK_DIR/stress.log" 2>&1 &
STRESS_PID=$!

# Poll for completion with 60s timeout
elapsed=0
while kill -0 "$STRESS_PID" 2>/dev/null; do
    if [ $elapsed -ge 60 ]; then
        echo "🔥 BUG DETECTED: Multithreaded HANG (Timed out after 60s)"
        kill -9 "$STRESS_PID" 2>/dev/null || true
        cat "$WORK_DIR/stress.log" | tail -5
        exit 1
    fi
    sleep 1
    elapsed=$((elapsed + 1))
done

wait "$STRESS_PID"
EXIT_CODE=$?
if [ $EXIT_CODE -eq 0 ]; then
    echo "✅ Test Finished (No Hang detected)"
    head -n 5 "$WORK_DIR/stress.log"
else
    echo "❌ Stress test exited with code $EXIT_CODE"
    cat "$WORK_DIR/stress.log" | tail -10
    exit $EXIT_CODE
fi

echo "✅ Test Finished: No deadlock detected."
exit 0
