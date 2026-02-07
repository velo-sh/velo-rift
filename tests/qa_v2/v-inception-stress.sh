#!/bin/bash
# ============================================================================
# Value Proof: The Virtual Toolchain Factory (Bug Diagnostic Version)
# ============================================================================
# Core Goal: Prove Phase 7 Architectural Deviations:
# 1. Hex Case Mismatch (Lowercase vs Uppercase)
# 2. Path Resolution Inconsistency (Relative vs Absolute)
# 3. VFS Projection Failure under concurrency
# ============================================================================

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

# Color helpers
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
NC='\033[0m'

echo "----------------------------------------------------------------"
echo -e "${BLUE}🌀 Phase 7 Diagnostic: The Virtual Toolchain Factory${NC}"
echo "----------------------------------------------------------------"

WORK_DIR="/tmp/vrift_endgame_stress_$$"
export VR_THE_SOURCE="$WORK_DIR/cas"
export VRIFT_SOCKET_PATH="$WORK_DIR/vrift.sock"
DAEMON_PID=""

cleanup() {
    [ -n "$DAEMON_PID" ] && kill -9 "$DAEMON_PID" 2>/dev/null || true
    rm -f "$VRIFT_SOCKET_PATH"
    rm -rf "$WORK_DIR" 2>/dev/null || true
}
trap cleanup EXIT

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/factory/p1" "$WORK_DIR/cas"

# 1. Setup Dataset
echo "📦 Generating 5,000 shared blobs..."
for i in {1..5000}; do
    echo "content_$i" > "$WORK_DIR/factory/p1/file_$i.txt"
done

# 2. Ingestion (Phantom Mode)
echo "⚡ Ingesting Project 1 (Phantom Mode)..."
cd "$WORK_DIR/factory/p1"
"$VRIFT_BIN" init . >/dev/null 2>&1
"$VRIFT_BIN" ingest . --mode phantom --output .vrift/manifest.lmdb >/dev/null 2>&1

# 3. Diagnostic Layer 1: Physical CAS Audit
echo -e "\n🔍 [Diagnostic 1] Physical CAS Audit..."
CAS_FILE=$(find "$WORK_DIR/cas/blake3" -type f 2>/dev/null | head -n 1)
if [ -n "$CAS_FILE" ]; then
    echo -e "   ${GREEN}✅ Physical Blobs exist in CAS.${NC}"
    echo "   Example: $CAS_FILE"

    # Check for Uppercase vs Lowercase bug
    if [[ "$CAS_FILE" =~ [A-Z] ]]; then
        echo -e "   ${YELLOW}⚠️  WARNING: Uppercase hex detected in CAS path. This may break case-sensitive lookups.${NC}"
    else
        echo -e "   ${GREEN}✅ CAS paths use lowercase hex.${NC}"
    fi
else
    echo -e "   ${RED}❌ FATAL: CAS is empty after ingestion!${NC}"
    exit 1
fi

# 4. Diagnostic Layer 2: Manifest Audit
echo -e "\n🔍 [Diagnostic 2] LMDB Manifest Audit..."
if [ -d "$WORK_DIR/factory/p1/.vrift/manifest.lmdb" ]; then
    echo -e "   ${GREEN}✅ LMDB Manifest directory exists.${NC}"
else
    echo -e "   ${RED}❌ FATAL: Manifest missing at .vrift/manifest.lmdb${NC}"
    exit 1
fi

# 5. Diagnostic Layer 3: Daemon Awareness
echo -e "\n🔍 [Diagnostic 3] Daemon Global Index Audit..."
# Run daemon with INFO logging
RUST_LOG=info "$VRIFTD_BIN" start > "$WORK_DIR/vriftd.log" 2>&1 &
DAEMON_PID=$!

# Wait for socket
waited=0
while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
    sleep 0.5
    waited=$((waited + 1))
done

if [ ! -S "$VRIFT_SOCKET_PATH" ]; then
    echo -e "   ${RED}❌ Daemon failed to start (no socket after 5s)${NC}"
    cat "$WORK_DIR/vriftd.log" 2>/dev/null | tail -10
    exit 1
fi

BLOBS_REPORTED=$("$VRIFT_BIN" status 2>/dev/null | grep "Unique blobs" | awk '{print $NF}' | tr -d ',') || true
if [ "$BLOBS_REPORTED" == "0" ] || [ -z "$BLOBS_REPORTED" ]; then
    echo -e "   ${RED}❌ BUG DETECTED: Daemon reports 0 blobs despite physical presence in CAS.${NC}"
    echo "   --- Recent Daemon Logs ---"
    tail -n 10 "$WORK_DIR/vriftd.log"
    echo "   --------------------------"
else
    echo -e "   ${GREEN}✅ Daemon successfully indexed $BLOBS_REPORTED blobs.${NC}"
fi

# 6. High-Concurrency Stress (Failure Proof)
echo -e "\n🔥 Triggering 100 concurrent VFS open waves..."
export VRIFT_MANIFEST="$WORK_DIR/factory/p1/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$WORK_DIR/factory/p1"
export VRIFT_VFS_PREFIX="$WORK_DIR/factory/p1"

PIDS=()
for i in {1..100}; do
    (
        env DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 \
            VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
            VRIFT_VFS_PREFIX="$WORK_DIR/factory/p1" VRIFT_PROJECT_ROOT="$WORK_DIR/factory/p1" \
            cat "$WORK_DIR/factory/p1/file_$(( (RANDOM % 5000) + 1 )).txt" > /dev/null 2>&1
    ) &
    PIDS+=($!)
done

WAVE_FAIL=0
for pid in "${PIDS[@]}"; do
    wait "$pid" 2>/dev/null || WAVE_FAIL=$((WAVE_FAIL + 1))
done

if [ $WAVE_FAIL -gt 0 ]; then
    echo -e "   ${RED}❌ FAILURE: $WAVE_FAIL of 100 concurrent VFS operations failed.${NC}"
else
    echo -e "   ${GREEN}✅ Success: Concurrent VFS operations completed.${NC}"
fi

# Cleanup handled by trap
echo "----------------------------------------------------------------"
echo -e "${YELLOW}🏁 Diagnostic Run Complete${NC}"
echo "----------------------------------------------------------------"
exit 0
