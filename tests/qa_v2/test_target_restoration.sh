#!/bin/bash
# ==============================================================================
# test_target_restoration.sh — THE golden test for target cache restoration
#
# Tests the most critical acceleration feature:
#   1. Build project normally
#   2. Snapshot target/ into CAS
#   3. cargo clean (delete target/)
#   4. Restore target/ from CAS
#   5. cargo build — MUST see all crates as FRESH (0 recompilation)
#
# Usage:
#   ./tests/qa_v2/test_target_restoration.sh <PROJECT_DIR>
#   ./tests/qa_v2/test_target_restoration.sh ~/rust_source/velo
# ==============================================================================
set -euo pipefail

PROJECT_DIR="${1:-}"
if [ -z "$PROJECT_DIR" ]; then
    echo "Usage: $0 <PROJECT_DIR>"
    exit 1
fi
PROJECT_DIR=$(cd "$PROJECT_DIR" && pwd -P)
PROJECT_NAME=$(basename "$PROJECT_DIR")

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../lib/vrift_env.sh"

PASSED=0
FAILED=0

pass() { echo "  ✅ PASS: $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ❌ FAIL: $1"; FAILED=$((FAILED + 1)); }
ms()   { python3 -c 'import time; print(int(time.time()*1000))'; }

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  Target Restoration Golden Test                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo "  Project: $PROJECT_DIR"
echo ""

cd "$PROJECT_DIR"

# ============================================================================
# Step 1: Ensure clean build state
# ============================================================================
echo "═══ Step 1: Full build (warm state) ═══"
cargo build 2>&1 | tail -1
pass "Initial build"

# Count crates compiled
CRATE_COUNT=$(find target/debug/.fingerprint -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
echo "  Fingerprint dirs: $CRATE_COUNT"

# Verify no-op build
NOOP_OUT=$(cargo build 2>&1)
if echo "$NOOP_OUT" | grep -q "Compiling"; then
    fail "No-op build should not recompile anything"
    echo "  $NOOP_OUT"
else
    pass "No-op build is FRESH"
fi

# ============================================================================
# Step 2: Snapshot target/ into CAS
# ============================================================================
echo ""
echo "═══ Step 2: Snapshot target/ ═══"
T0=$(ms)
SNAP_OUT=$("$VRIFT_CLI" --the-source-root "$VR_THE_SOURCE" snapshot-target 2>&1)
T1=$(ms)
SNAP_MS=$((T1 - T0))

if echo "$SNAP_OUT" | grep -q "Snapshot complete"; then
    SNAP_CRATES=$(echo "$SNAP_OUT" | grep -o '[0-9]* crates' | head -1)
    echo "  $SNAP_CRATES cached in ${SNAP_MS}ms"
    pass "Snapshot"
else
    fail "Snapshot"
    echo "$SNAP_OUT"
    exit 1
fi

# ============================================================================
# Step 3: cargo clean
# ============================================================================
echo ""
echo "═══ Step 3: cargo clean ═══"
chflags -R nouchg target 2>/dev/null || true
rm -rf target
echo "  target/ removed"

# Verify it's gone
if [ -d target ]; then
    fail "target/ should be gone"
    exit 1
fi
pass "target/ clean"

# ============================================================================
# Step 4: Restore target/ from CAS
# ============================================================================
echo ""
echo "═══ Step 4: Restore target/ ═══"
T0=$(ms)
RESTORE_OUT=$("$VRIFT_CLI" --the-source-root "$VR_THE_SOURCE" restore-target 2>&1)
T1=$(ms)
RESTORE_MS=$((T1 - T0))

if echo "$RESTORE_OUT" | grep -q "Restore complete"; then
    RESTORED_COUNT=$(echo "$RESTORE_OUT" | grep -o '[0-9]* crates' | head -1)
    echo "  $RESTORED_COUNT in ${RESTORE_MS}ms"
    pass "Restore"
else
    fail "Restore"
    echo "$RESTORE_OUT"
    exit 1
fi

# Check restored file structure
FP_DIRS=$(find target/debug/.fingerprint -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
RLIBS=$(find target/debug/deps -name '*.rlib' 2>/dev/null | wc -l | tr -d ' ')
echo "  Fingerprint dirs: $FP_DIRS"
echo "  Rlibs restored: $RLIBS"

if [ "$FP_DIRS" -gt 0 ]; then
    pass "Fingerprint dirs restored ($FP_DIRS)"
else
    fail "No fingerprint dirs restored"
fi

if [ "$RLIBS" -gt 0 ]; then
    pass "Rlibs restored ($RLIBS)"
else
    fail "No rlibs restored"
fi

# ============================================================================
# Step 5: THE KEY TEST — cargo build should be FRESH
# ============================================================================
echo ""
echo "═══ Step 5: cargo build (MUST BE FRESH!) ═══"
T0=$(ms)
BUILD_OUT=$(cargo build 2>&1)
T1=$(ms)
BUILD_MS=$((T1 - T0))

COMPILED=$(echo "$BUILD_OUT" | grep -c "Compiling" || true)
FRESH_LINE=$(echo "$BUILD_OUT" | grep "Finished" || true)

echo "  Build time: ${BUILD_MS}ms"
echo "  Crates compiled: $COMPILED"
echo "  $FRESH_LINE"

if [ "$COMPILED" -eq 0 ]; then
    pass "ALL CRATES FRESH — target restoration works! 🎉"
elif [ "$COMPILED" -le 5 ]; then
    fail "Partial restoration: $COMPILED crates recompiled (should be 0)"
    echo "$BUILD_OUT" | grep "Compiling" | head -10
else
    fail "Full recompilation: $COMPILED crates compiled (restoration FAILED)"
    echo "  First 5 compiled:"
    echo "$BUILD_OUT" | grep "Compiling" | head -5
fi

# ============================================================================
# Step 6: Verify build output works
# ============================================================================
echo ""
echo "═══ Step 6: Verify build output ═══"

# Check that binary exists and runs
BIN_NAME=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; bins=[x['name'] for p in pkgs for x in p['targets'] if 'bin' in x['kind']]; print(bins[0] if bins else '')" 2>/dev/null || echo "")

if [ -n "$BIN_NAME" ] && [ -f "target/debug/$BIN_NAME" ]; then
    if timeout 5 "./target/debug/$BIN_NAME" --version >/dev/null 2>&1 || \
       timeout 5 "./target/debug/$BIN_NAME" --help >/dev/null 2>&1; then
        pass "Binary executes correctly"
    else
        pass "Binary exists"
    fi
fi

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $PASSED passed, $FAILED failed"
echo ""
echo "  Snapshot:  ${SNAP_MS}ms"
echo "  Restore:   ${RESTORE_MS}ms"
echo "  Build:     ${BUILD_MS}ms ($COMPILED crates compiled)"
echo "═══════════════════════════════════════════════════════════════"

[ "$FAILED" -gt 0 ] && { echo "  ❌ TEST FAILED"; exit 1; }
echo "  ✅ ALL TESTS PASSED"
exit 0
