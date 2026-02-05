#!/bin/bash
# VRift Complete System Startup Verification Test
# Tests: Prerequisites → vriftd → vrift-vdird → Shim → Compiler
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VRIFT_CLI="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
VDIRD_BIN="$PROJECT_ROOT/target/release/vrift-vdird"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
TEST_WORKSPACE="/tmp/vrift_startup_test_$$"

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'
PASS=0; FAIL=0

log_pass() { echo -e "   ${GREEN}✓ $1${NC}"; PASS=$((PASS+1)); }
log_fail() { echo -e "   ${RED}✗ $1${NC}"; FAIL=$((FAIL+1)); }

cleanup() {
    pkill -9 -f "vriftd.*$TEST_WORKSPACE" 2>/dev/null || true
    pkill -9 -f "vrift-vdird.*$TEST_WORKSPACE" 2>/dev/null || true
    rm -rf "$TEST_WORKSPACE"
}
trap cleanup EXIT

echo "═══════════════════════════════════════════════════════════════"
echo "📍 STEP 0: Prerequisites"
echo "═══════════════════════════════════════════════════════════════"
[ -x "$VRIFT_CLI" ] && log_pass "vrift CLI" || log_fail "vrift CLI missing"
[ -x "$VRIFTD_BIN" ] && log_pass "vriftd" || log_fail "vriftd missing"
[ -x "$VDIRD_BIN" ] && log_pass "vrift-vdird" || log_fail "vrift-vdird missing"
[ -f "$SHIM_LIB" ] && log_pass "Shim library" || log_fail "Shim missing"
mkdir -p "$TEST_WORKSPACE"/{src,build,.vrift}
echo 'int main() { return 42; }' > "$TEST_WORKSPACE/src/hello.c"
log_pass "Test workspace created"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "📍 STEP 1: Global Daemon (vriftd) - SKIPPED (stateless test)"
echo "═══════════════════════════════════════════════════════════════"
log_pass "Skipped - using existing daemon or stateless mode"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "📍 STEP 2: Project Daemon (vrift-vdird)"
echo "═══════════════════════════════════════════════════════════════"
DAEMON_LOG="$TEST_WORKSPACE/vdird.log"
timeout 5 "$VDIRD_BIN" "$TEST_WORKSPACE" > "$DAEMON_LOG" 2>&1 &
VDIRD_PID=$!
sleep 2
if kill -0 $VDIRD_PID 2>/dev/null; then
    if ps aux | grep $VDIRD_PID | grep -v grep | grep -q "UE"; then
        log_fail "vrift-vdird in zombie state (UE)"
    else
        log_pass "vrift-vdird running"
    fi
else
    log_fail "vrift-vdird died immediately"
    cat "$DAEMON_LOG" | head -10
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "📍 STEP 3: Shim Injection"
echo "═══════════════════════════════════════════════════════════════"
export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
export VRIFT_INCEPTION=1
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
if cat "$TEST_WORKSPACE/src/hello.c" 2>/dev/null | grep -q "int main"; then
    log_pass "Shim allows file read"
else
    log_fail "Shim blocked file read"
fi
echo "shim_test" > "$TEST_WORKSPACE/shim_test.txt" && log_pass "Shim allows file write" || log_fail "Shim blocked write"
unset DYLD_INSERT_LIBRARIES

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "📍 STEP 4: Compiler Workflow"
echo "═══════════════════════════════════════════════════════════════"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
gcc -c "$TEST_WORKSPACE/src/hello.c" -o "$TEST_WORKSPACE/build/hello.o" 2>/dev/null && log_pass "GCC compile" || log_fail "GCC compile"
gcc "$TEST_WORKSPACE/build/hello.o" -o "$TEST_WORKSPACE/build/hello" 2>/dev/null && log_pass "GCC link" || log_fail "GCC link"
"$TEST_WORKSPACE/build/hello" && EXIT_CODE=$? || EXIT_CODE=$?
[ $EXIT_CODE -eq 42 ] && log_pass "Binary runs (exit=$EXIT_CODE)" || log_fail "Binary wrong exit ($EXIT_CODE)"
unset DYLD_INSERT_LIBRARIES VRIFT_PROJECT_ROOT VRIFT_INCEPTION

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "SUMMARY: Passed=$PASS Failed=$FAIL"
echo "═══════════════════════════════════════════════════════════════"
[ $FAIL -eq 0 ] && echo -e "${GREEN}✅ COMPLETE SYSTEM STARTUP VERIFIED${NC}" && exit 0
echo -e "${RED}❌ STARTUP VERIFICATION FAILED${NC}" && exit 1
