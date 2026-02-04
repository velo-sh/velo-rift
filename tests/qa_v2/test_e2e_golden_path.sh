#!/bin/bash
# ==============================================================================
# Velo Rift E2E "Golden Path" - Full Lifecycle Verification
# ==============================================================================
# This script covers the longest and widest UX path:
# Service -> Init -> Ingest -> Inception -> Mutate -> Wake -> Status
# ==============================================================================

set -e

# Configuration
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
VFS_PREFIX="/tmp/vrift_gold_$$"
SRC_DATA="$VFS_PREFIX/src"
WORK_DIR="$VFS_PREFIX/project"
CAS_ROOT="$VFS_PREFIX/cas"

# Platform detection
OS=$(uname -s)
if [ "$OS" == "Darwin" ]; then
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
    VFS_ENV_BASE="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1"
else
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.so"
    VFS_ENV_BASE="LD_PRELOAD=$SHIM_LIB"
fi

# Compile helpers
for tool in mini_read mini_mkdir; do
    if [ ! -f "$PROJECT_ROOT/tests/qa_v2/$tool" ] || [ "$PROJECT_ROOT/tests/qa_v2/$tool.c" -nt "$PROJECT_ROOT/tests/qa_v2/$tool" ]; then
        cc "$PROJECT_ROOT/tests/qa_v2/$tool.c" -o "$PROJECT_ROOT/tests/qa_v2/$tool"
    fi
done

# Clean UI
echo "----------------------------------------------------------------"
echo "🌀 Velo Rift: Starting E2E Golden Path Verification"
echo "----------------------------------------------------------------"

# 1. Environment Preparation
mkdir -p "$SRC_DATA" "$WORK_DIR" "$CAS_ROOT"
echo "Hello Velo" > "$SRC_DATA/hello.txt"
mkdir -p "$SRC_DATA/subdir"
echo "Nested Data" > "$SRC_DATA/subdir/nested.txt"

# 2. Service Lifecycle (Phase A)
echo "🚀 [Phase A] Background Service Installation..."
pkill vriftd || true
# Socket might be in VR_THE_SOURCE or default /tmp
rm -f /tmp/vrift.sock || true

# Try service install, but fallback to manual start if it fails (common in Docker)
if ! $VRIFT_BIN --the-source-root "$CAS_ROOT" service install 2>/dev/null; then
    echo "⚠️  Service install failed (possibly no systemd). Falling back to manual start..."
    # Start vriftd in background
    vriftd_bin="$(dirname "$VRIFT_BIN")/vriftd"
    VR_THE_SOURCE="$CAS_ROOT" "$vriftd_bin" start > /tmp/vriftd.log 2>&1 &
    sleep 2
fi

# Wait for socket to be created (launchd takes a moment to start the daemon)
echo "   Waiting for service socket..."
for i in {1..10}; do
    if [ -S "/tmp/vrift.sock" ]; then
        break
    fi
    sleep 0.5
done

# Verify service is running
if ! pgrep vriftd >/dev/null && [ ! -S "/tmp/vrift.sock" ]; then 
    echo "❌ Service failed to start"
    [ -f /tmp/vriftd.log ] && cat /tmp/vriftd.log
    exit 1
fi
echo "✅ Service running."

# 3. Project Onboarding (Phase B)
echo "📂 [Phase B] Project Initialization & Ingestion..."
cd "$WORK_DIR"
$VRIFT_BIN --the-source-root "$CAS_ROOT" init > /dev/null
$VRIFT_BIN --the-source-root "$CAS_ROOT" ingest "$SRC_DATA" --output "$WORK_DIR/.vrift/manifest.lmdb"

if [ ! -d ".vrift/manifest.lmdb" ]; then
    echo "❌ Ingestion failed: Manifests not generated"
    exit 1
fi
echo "✅ Project initialized and data ingested (Solid Mode)."

# 4. Deep Virtualization (Phase C)
echo "🌀 [Phase C] VFS Inception & Mutation Audit..."
MINI_READ="$PROJECT_ROOT/tests/qa_v2/mini_read"
MINI_MKDIR="$PROJECT_ROOT/tests/qa_v2/mini_mkdir"

VFS_ENV="$VFS_ENV_BASE VRIFT_MANIFEST=$WORK_DIR/.vrift/manifest.lmdb VR_THE_SOURCE=$CAS_ROOT VRIFT_PROJECT_ROOT=$WORK_DIR VRIFT_VFS_PREFIX=$WORK_DIR VRIFT_DEBUG=1"

echo "   Testing Virtual Read..."
# Using mini_read to bypass SIP issues
if ! env $VFS_ENV VRIFT_LOG=trace "$MINI_READ" "$WORK_DIR/src/hello.txt" > "$WORK_DIR/e2e_read.log" 2>&1; then
    echo "❌ VFS Read Failure (Exit Code: $?)"
    cat "$WORK_DIR/e2e_read.log"
    exit 1
fi
cat "$WORK_DIR/e2e_read.log"
if ! grep -q "Hello Velo" "$WORK_DIR/e2e_read.log"; then
    echo "❌ VFS Content Mismatch"
    exit 1
fi
echo "✅ Virtual File Read: Passed"

echo "   Testing Virtual Mutation (mkdir)..."
# Using mini_mkdir to bypass SIP issues
# Expected: Operation not permitted (EPERM) because mutation perimeter blocks it
if env $VFS_ENV "$MINI_MKDIR" "$WORK_DIR/src/new_dir" 2>&1 | grep -q "Operation not permitted"; then
    echo "✅ Virtual Mutation Blocked (Mutation Perimeter): Passed"
else
    echo "❌ Virtual Mutation Perimeter Failure: Mutation was not blocked as expected"
    exit 1
fi

# 5. Persistence & Recovery (Phase D)
echo "💾 [Phase D] Persistence & Session Recovery..."
# Check if session is tracked
if ! (cd "$WORK_DIR" && $VRIFT_BIN status -s) | grep "Session: ● \[Solid\] Active"; then
    echo "❌ Session not active"
    exit 1
fi
echo "✅ Session tracking: Passed"

# 6. Global Stats (Phase E)
echo "📊 [Phase E] Global Health Check..."
if ! $VRIFT_BIN --the-source-root "$CAS_ROOT" status | grep "Unique blobs"; then
    echo "❌ Global Health Check Failed"
    exit 1
fi
echo "✅ Global CAS analysis: Passed"

# 7. Teardown
echo "🧽 [Phase F] Final Teardown..."
$VRIFT_BIN service uninstall || true
# rm -rf "$VFS_PREFIX"

echo "----------------------------------------------------------------"
echo "🏁 GOLDEN PATH COMPLETE: 100% Success"
echo "----------------------------------------------------------------"
