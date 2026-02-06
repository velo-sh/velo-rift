#!/bin/bash
# ============================================================================
# Phase 8: Path & Hex Normalization Invariants (Adversarial Suite v2)
# ============================================================================
# Objective: Prove that the VFS projection layer itself is brittly Case-Sensitive
# or lacks proper normalization even when the host is case-insensitive.
# ============================================================================

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

# Platform detection
OS=$(uname -s)
if [ "$OS" == "Darwin" ]; then
    VFS_ENV="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1"
else
    VFS_ENV="LD_PRELOAD=${SHIM_LIB/dylib/so}"
fi

# Color helpers
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
NC='\033[0m'

echo "----------------------------------------------------------------"
echo -e "${BLUE}🧪 Phase 8: VFS Case Sensitivity & Normalization Proof${NC}"
echo "----------------------------------------------------------------"

WORK_DIR="/tmp/vrift_normalization_v2"
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/factory/p1" "$WORK_DIR/cas"

# 1. VFS Level Case-Sensitivity Proof (OS-Independent)
# ----------------------------------------------------------------
echo -e "\n🔥 [Test 1] VFS Case-Sensitivity Invariant Proof..."
cd "$WORK_DIR/factory/p1"
echo "sensitive content" > "CaseFile.txt"

# Ingest with exact casing
"$VRIFT_BIN" --the-source-root "$WORK_DIR/cas" ingest . --mode phantom >/dev/null 2>&1

# Start Daemon
export VR_THE_SOURCE="$WORK_DIR/cas"
"$VRIFTD_BIN" start > "$WORK_DIR/vriftd.log" 2>&1 &
DAEMON_PID=$!
sleep 3

export VRIFT_MANIFEST="$WORK_DIR/factory/p1/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$WORK_DIR/factory/p1"
export VRIFT_VFS_PREFIX="$WORK_DIR/factory/p1"

echo "   Manifest contains: CaseFile.txt"
echo "   Attempting access via lowercase: casefile.txt"

if ! env $VFS_ENV cat "casefile.txt" > /dev/null 2>&1; then
    echo -e "   ${GREEN}✅ PROVED: VFS is strictly case-sensitive even on case-insensitive hosts.${NC}"
    echo "   (This verifies the discrepancy between Host and VRift policies)"
else
    echo -e "   ${YELLOW}⚠️  WARNING: VFS successfully resolved lowercase path. This implies auto-normalization or host-leakage.${NC}"
fi

# 2. Path Canonicalization Proof (The "Double Dot" Trap)
# ----------------------------------------------------------------
echo -e "\n🔥 [Test 2] Relative Path Canonicalization Dead-End..."
echo "   Attempting access via complex relative path: ../p1/CaseFile.txt"
if ! env $VFS_ENV cat "../p1/CaseFile.txt" > /dev/null 2>&1; then
    echo -e "   ${GREEN}✅ PROVED: VFS fails to resolve non-canonical relative paths.${NC}"
else
    echo -e "   ${RED}❌ FAILURE: VFS successfully resolved complex relative path.${NC}"
fi

# Cleanup
kill $DAEMON_PID 2>/dev/null || true
echo "----------------------------------------------------------------"
echo -e "${YELLOW}🏁 Phase 8 Diagnostic Complete${NC}"
echo "----------------------------------------------------------------"
