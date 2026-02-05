#!/bin/bash
# ==============================================================================
# Test: FS Watch Race Condition (Stress Test)
# ==============================================================================
# This test attempts to trigger the FS Watch race condition by:
# 1. Rapidly creating and deleting temp files during daemon startup
# 2. Running multiple iterations to increase chance of hitting the race
#
# BUG: FS Watch tries to watch .tmp files that get deleted, causing ENOENT
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VDIRD_BIN="$PROJECT_ROOT/target/release/vrift-vdird"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

ITERATIONS=10
BUG_TRIGGERED=0
PASS_COUNT=0

echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  TEST: FS Watch Race Condition (Stress Test)                          ║"
echo "║  Running $ITERATIONS iterations to trigger race condition                       ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""

for i in $(seq 1 $ITERATIONS); do
    TEST_DIR="/tmp/test_fs_watch_stress_${$}_${i}"
    DAEMON_LOG="$TEST_DIR/daemon.log"
    
    # Create test dir
    mkdir -p "$TEST_DIR/.vrift"
    
    # Start temp file storm in background (creates race condition)
    (
        for j in $(seq 1 20); do
            touch "$TEST_DIR/.vrift/temp_${j}.tmp" 2>/dev/null || true
            rm -f "$TEST_DIR/.vrift/temp_${j}.tmp" 2>/dev/null || true
        done
    ) &
    STORM_PID=$!
    
    # Start daemon during the storm
    "$VDIRD_BIN" "$TEST_DIR" > "$DAEMON_LOG" 2>&1 &
    VDIRD_PID=$!
    
    # Wait for daemon to initialize or fail
    sleep 2
    
    # Kill storm and daemon
    kill $STORM_PID 2>/dev/null || true
    kill -9 $VDIRD_PID 2>/dev/null || true
    wait $STORM_PID 2>/dev/null || true
    wait $VDIRD_PID 2>/dev/null || true
    
    # Check for FS Watch failure
    if grep -q "Failed to start FS watch\|Watch exited" "$DAEMON_LOG" 2>/dev/null; then
        echo -e "   Iteration $i: ${RED}❌ BUG TRIGGERED${NC}"
        BUG_TRIGGERED=$((BUG_TRIGGERED + 1))
        
        # Save evidence
        if [ $BUG_TRIGGERED -eq 1 ]; then
            cp "$DAEMON_LOG" /tmp/fs_watch_bug_evidence.log
            echo "   📁 Evidence saved to /tmp/fs_watch_bug_evidence.log"
        fi
    else
        echo -e "   Iteration $i: ${GREEN}✓ stable${NC}"
        PASS_COUNT=$((PASS_COUNT + 1))
    fi
    
    # Cleanup
    rm -rf "$TEST_DIR"
done

# Summary
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 STRESS TEST RESULTS"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "   Total iterations: $ITERATIONS"
echo "   Passed:           $PASS_COUNT"
echo "   Bug triggered:    $BUG_TRIGGERED"
echo ""

if [ $BUG_TRIGGERED -gt 0 ]; then
    echo -e "${RED}❌ BUG CONFIRMED: FS Watch race condition reproduced ($BUG_TRIGGERED/$ITERATIONS)${NC}"
    echo ""
    echo "Evidence log:"
    echo "───────────────────────────────────────"
    cat /tmp/fs_watch_bug_evidence.log 2>/dev/null | head -30
    echo "───────────────────────────────────────"
    echo ""
    echo "Root cause: FS Watch tries to watch temp files that are deleted"
    echo "Fix needed: Add filter in watch.rs to ignore .tmp/.swp files"
    exit 1
else
    echo -e "${GREEN}✅ PASS: No race condition triggered in $ITERATIONS iterations${NC}"
    echo ""
    echo "Note: Race condition is timing-dependent. Consider:"
    echo "  - Running with higher ITERATIONS value"
    echo "  - Running under CPU load"
    echo "  - Adding a fix in watch.rs as a precaution"
    exit 0
fi
