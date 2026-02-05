#!/bin/bash
# ==============================================================================
# BUG TEST #1: FS Watch Crashes on Transient ENOENT
# ==============================================================================
# PROVES: notify library crash when file deleted during watch init
# ROOT CAUSE: watch.rs Line 156-159 returns on ANY error
#
# This is a RACE CONDITION bug, not a .tmp file bug.
# NOTE: Uses 3-second timeout per iteration to avoid hanging (BUG #3)
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VDIRD_BIN="$PROJECT_ROOT/target/release/vrift-vdird"
TIMEOUT_SEC=3

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  BUG TEST #1: FS Watch Crashes on Transient ENOENT                    ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""

# Run 5 iterations with file deletion storm
ITERATIONS=5
BUG_COUNT=0

for i in $(seq 1 $ITERATIONS); do
    TEST_DIR="/tmp/test_enoent_bug_${$}_${i}"
    mkdir -p "$TEST_DIR/src"
    
    # Create many files, then delete them rapidly (not in .vrift!)
    (
        for j in $(seq 1 30); do
            touch "$TEST_DIR/src/temp_${j}.c" 2>/dev/null || true
            rm -f "$TEST_DIR/src/temp_${j}.c" 2>/dev/null || true
        done
    ) &
    STORM_PID=$!
    
    # Start daemon during storm (with timeout to prevent hang - BUG #3)
    "$VDIRD_BIN" "$TEST_DIR" > "$TEST_DIR/log" 2>&1 &
    VDIRD_PID=$!
    
    # Timeout watchdog
    ( sleep $TIMEOUT_SEC && kill -9 $VDIRD_PID 2>/dev/null ) &
    WATCHDOG_PID=$!
    sleep 2
    
    kill $STORM_PID 2>/dev/null || true
    kill -9 $VDIRD_PID 2>/dev/null || true
    kill $WATCHDOG_PID 2>/dev/null || true
    wait 2>/dev/null || true
    
    if grep -q "Watch exited\|Failed to start FS watch" "$TEST_DIR/log" 2>/dev/null; then
        echo -e "   Iteration $i: ${RED}❌ BUG: Watch crashed on ENOENT${NC}"
        [ $BUG_COUNT -eq 0 ] && cp "$TEST_DIR/log" /tmp/enoent_bug_evidence.log
        BUG_COUNT=$((BUG_COUNT + 1))
    else
        echo -e "   Iteration $i: ${GREEN}✓ stable${NC}"
    fi
    
    rm -rf "$TEST_DIR"
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [ $BUG_COUNT -gt 0 ]; then
    echo -e "${RED}❌ BUG #1 CONFIRMED: Watch crashes on transient ENOENT ($BUG_COUNT/$ITERATIONS)${NC}"
    echo ""
    echo "Evidence: grep 'ENOENT\|Watch exited' /tmp/enoent_bug_evidence.log"
    exit 1
else
    echo -e "${GREEN}✅ PASS: No crash in $ITERATIONS iterations${NC}"
    exit 0
fi
