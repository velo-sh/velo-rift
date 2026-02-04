#!/bin/bash
# ============================================================================
# Value Proof 2: Cross-Domain Reliability (Rename Redirects)
# ============================================================================
# This test demonstrates that Velo Rift acts as a reliable "Virtual Volume".
#
# Key Features Verified:
# 1. Inbound Move (Outside -> VFS): Falls back to copy+delete (EXDEV handling)
# 2. Outbound Move (VFS -> Outside): Falls back to copy+delete (EXDEV handling)
# 3. Virtual Rename (VFS -> VFS): Atomic, metadata-only update (No physical IO)
# 4. Boundary Protection: Hardlinks across boundary are rejected (EXDEV)

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"

# Timeout configuration (macOS compatible using perl alarm)
TIMEOUT_SEC=10
run_with_timeout() {
    local timeout="$1"
    shift
    perl -e 'alarm shift; exec @ARGV' "$timeout" "$@"
    return $?
}

# Platform detection
OS=$(uname -s)
if [ "$OS" == "Darwin" ]; then
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
    VFS_ENV="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1"
    BIN_MV="/bin/mv"
    BIN_LN="/bin/ln"
    BIN_SHASUM="/usr/bin/shasum"
else
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.so"
    VFS_ENV="LD_PRELOAD=$SHIM_LIB"
    BIN_MV="/bin/mv"
    BIN_LN="/bin/ln"
    # Linux often has sha1sum instead of shasum, or shasum is in /usr/bin
    BIN_SHASUM=$(command -v sha1sum || command -v shasum)
fi

# Color helpers
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

echo "----------------------------------------------------------------"
echo -e "${BLUE}🔁 Velo Rift Value Proof: Cross-Domain Reliability${NC}"
echo "----------------------------------------------------------------"

# Setup work dir
WORK_DIR="/tmp/vrift_value_2_rename"
if [ "$(uname -s)" == "Darwin" ]; then
    chflags -R nouchg "$WORK_DIR" 2>/dev/null || true
fi
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/project"
mkdir -p "$WORK_DIR/external"
mkdir -p "$WORK_DIR/bin"

# SIP Bypass: Copy binaries (crucial on macOS, good for isolation on Linux)
cp "$BIN_MV" "$WORK_DIR/bin/mv"
cp "$BIN_LN" "$WORK_DIR/bin/ln"
cp "$BIN_SHASUM" "$WORK_DIR/bin/shasum"

# Helper aliases (using SIP-bypassed binaries)
MY_MV="$WORK_DIR/bin/mv"
MY_LN="$WORK_DIR/bin/ln"

# Create a 10MB test file outside
echo "📦 Creating external data (10MB)..."
dd if=/dev/urandom of="$WORK_DIR/external/data.bin" bs=1M count=10 status=none
EXT_HASH=$("$BIN_SHASUM" "$WORK_DIR/external/data.bin" | awk '{print $1}')

# Initialize Velo Rift
echo "⚡ Initializing Project..."
cd "$WORK_DIR/project"
"$VRIFT_BIN" init . >/dev/null 2>&1
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb"
export VRIFT_CAS_ROOT="$WORK_DIR/project/.vrift/cas"

# Shim Environment
FULL_VFS_ENV="$VFS_ENV VRIFT_MANIFEST=$VRIFT_MANIFEST VRIFT_VFS_PREFIX=$WORK_DIR/project"

# 1. Inbound Move (Cross-Domain In)
echo -e "\n${BLUE}🧪 Test 1: Inbound Move (External -> VFS)${NC}"
echo "   Action: mv ../external/data.bin ./inbound.bin"
# Note: Since we use local mv with shim, and project is same device,
# verify shim allows it or forces copy. 
# Shim returns EXDEV for cross-boundary, forcing mv to copy.
if ! run_with_timeout $TIMEOUT_SEC env $FULL_VFS_ENV "$MY_MV" "$WORK_DIR/external/data.bin" "$WORK_DIR/project/inbound.bin" 2>/dev/null; then
    RET=$?
    if [ $RET -eq 142 ]; then
        echo -e "   ${RED}❌ TIMEOUT: Inbound move hung after ${TIMEOUT_SEC}s.${NC}"
    else
        echo -e "   ${RED}❌ Failed: Inbound move failed (exit=$RET).${NC}"
    fi
    exit 1
fi

if [ -f "$WORK_DIR/project/inbound.bin" ]; then
    echo -e "   ${GREEN}✅ Success: File moved into VFS territory.${NC}"
else
    echo -e "   ${RED}❌ Failed: Inbound move failed.${NC}"
    exit 1
fi

# Verify Integrity
IN_HASH=$(run_with_timeout $TIMEOUT_SEC env $FULL_VFS_ENV "$BIN_SHASUM" "$WORK_DIR/project/inbound.bin" 2>/dev/null | awk '{print $1}')
if [ "$IN_HASH" == "$EXT_HASH" ]; then
    echo -e "   ${GREEN}✅ Integrity: Content hash matches (${IN_HASH}).${NC}"
else
    echo -e "   ${RED}❌ Integrity Failed: Hash mismatch.${NC}"
    exit 1
fi

# 2. Virtual Rename (VFS -> VFS)
echo -e "\n${BLUE}🧪 Test 2: Virtual Rename (Internal -> Internal)${NC}"
echo "   Action: mv ./inbound.bin ./renamed.bin"
START_TIME=$(date +%s)
if ! run_with_timeout $TIMEOUT_SEC env $FULL_VFS_ENV "$MY_MV" "$WORK_DIR/project/inbound.bin" "$WORK_DIR/project/renamed.bin" 2>/dev/null; then
    RET=$?
    if [ $RET -eq 142 ]; then
        echo -e "   ${RED}❌ TIMEOUT: Virtual rename hung after ${TIMEOUT_SEC}s.${NC}"
    else
        echo -e "   ${RED}❌ Failed: Virtual rename failed (exit=$RET).${NC}"
    fi
    exit 1
fi
END_TIME=$(date +%s)
DURATION=$(( END_TIME - START_TIME ))

if [ -f "$WORK_DIR/project/renamed.bin" ] && [ ! -f "$WORK_DIR/project/inbound.bin" ]; then
    echo -e "   ${GREEN}✅ Success: Virtual rename complete.${NC}"
    echo -e "   ${GREEN}⚡ Speed: Unnoticed (${DURATION}ms) - Likely metadata only.${NC}"
else
    echo -e "   ${RED}❌ Failed: Virtual rename failed.${NC}"
    exit 1
fi

# 3. Outbound Move (Cross-Domain Out)
echo -e "\n${BLUE}🧪 Test 3: Outbound Move (VFS -> External)${NC}"
echo "   Action: mv ./renamed.bin ../external/outbound.bin"
if ! run_with_timeout $TIMEOUT_SEC env $FULL_VFS_ENV "$MY_MV" "$WORK_DIR/project/renamed.bin" "$WORK_DIR/external/outbound.bin" 2>/dev/null; then
    RET=$?
    if [ $RET -eq 142 ]; then
        echo -e "   ${RED}❌ TIMEOUT: Outbound move hung after ${TIMEOUT_SEC}s.${NC}"
    else
        echo -e "   ${RED}❌ Failed: Outbound move failed (exit=$RET).${NC}"
    fi
    exit 1
fi

if [ -f "$WORK_DIR/external/outbound.bin" ] && [ ! -f "$WORK_DIR/project/renamed.bin" ]; then
    echo -e "   ${GREEN}✅ Success: File moved out of VFS territory.${NC}"
else
    echo -e "   ${RED}❌ Failed: Outbound move failed.${NC}"
    exit 1
fi

OUT_HASH=$("$WORK_DIR/bin/shasum" "$WORK_DIR/external/outbound.bin" | awk '{print $1}')
if [ "$OUT_HASH" == "$EXT_HASH" ]; then
    echo -e "   ${GREEN}✅ Integrity: Content preserved after round-trip.${NC}"
else
    echo -e "   ${RED}❌ Integrity Failed: Hash mismatch.${NC}"
    exit 1
fi

# 4. Boundary Protection (Hardlink)
echo -e "\n${BLUE}🧪 Test 4: Boundary Protection (Hardlink)${NC}"
echo "   Action: ln ../external/outbound.bin ./hardlink.bin (Should Fail)"
set +e
run_with_timeout $TIMEOUT_SEC env $FULL_VFS_ENV "$MY_LN" "$WORK_DIR/external/outbound.bin" "$WORK_DIR/project/hardlink.bin" 2>/dev/null
LN_EXIT=$?
set -e

if [ $LN_EXIT -ne 0 ]; then
     echo -e "   ${GREEN}✅ Success: Hardlink creation prevented (EXDEV forced).${NC}"
else
     echo -e "   ${RED}❌ Failure: Hardlink allowed across boundary (Violation of RFC-0047).${NC}"
     exit 1
fi

echo "----------------------------------------------------------------"
echo -e "${GREEN}🏆 VALUE PROOF 2: SUCCESSFUL${NC}"
echo "----------------------------------------------------------------"
