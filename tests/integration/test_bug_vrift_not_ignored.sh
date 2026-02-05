#!/bin/bash
# ==============================================================================
# BUG TEST #2: .vrift Directory Not Ignored by FS Watch
# ==============================================================================
# PROVES: .vrift directory should be ignored but is NOT
# EXPECTED: FS Watch should NEVER process files under .vrift/
#
# If .vrift is properly ignored, daemon_state.tmp would never be watched
# NOTE: Uses 5-second timeout to avoid hanging (BUG #3: daemon can hang)
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
VDIRD_BIN="$PROJECT_ROOT/target/release/vrift-vdird"
TIMEOUT_SEC=5

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  BUG TEST #2: .vrift Directory Not Ignored by FS Watch                ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""

TEST_DIR="/tmp/test_vrift_ignore_$$"
DAEMON_LOG="$TEST_DIR/daemon.log"

cleanup() {
    pkill -9 -f "vrift-vdird.*$TEST_DIR" 2>/dev/null || true
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Setup
mkdir -p "$TEST_DIR/.vrift"
mkdir -p "$TEST_DIR/src"
echo "int main(){}" > "$TEST_DIR/src/main.c"

echo "📍 Starting vrift-vdird (with ${TIMEOUT_SEC}s timeout watchdog)..."
"$VDIRD_BIN" "$TEST_DIR" > "$DAEMON_LOG" 2>&1 &
VDIRD_PID=$!

# Timeout watchdog to prevent hanging (BUG #3)
( sleep $TIMEOUT_SEC && kill -9 $VDIRD_PID 2>/dev/null ) &
WATCHDOG_PID=$!

sleep 2

echo "📍 Creating files in .vrift directory (should be IGNORED)..."
echo "test1" > "$TEST_DIR/.vrift/test_file1.txt"
echo "test2" > "$TEST_DIR/.vrift/test_file2.txt"
touch "$TEST_DIR/.vrift/another.tmp"
sleep 1

echo "📍 Creating files in src directory (should be PROCESSED)..."
echo "// source" > "$TEST_DIR/src/app.c"
sleep 1

kill -9 $VDIRD_PID 2>/dev/null || true
kill $WATCHDOG_PID 2>/dev/null || true
wait $VDIRD_PID 2>/dev/null || true

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 ANALYSIS"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Check 1: Does .vrift appear in error messages?
echo "🔍 Check 1: .vrift files in error messages?"
if grep -q "\.vrift.*ENOENT\|No such file.*\.vrift\|Failed.*\.vrift" "$DAEMON_LOG" 2>/dev/null; then
    echo -e "   ${RED}❌ BUG: .vrift files caused errors (should be ignored!)${NC}"
    grep "\.vrift" "$DAEMON_LOG" | head -3 | sed 's/^/      /'
    VRIFT_IN_ERRORS=1
else
    echo -e "   ${GREEN}✓ No .vrift files in error messages${NC}"
    VRIFT_IN_ERRORS=0
fi

# Check 2: Does daemon process .vrift files at all?
echo ""
echo "🔍 Check 2: .vrift files being processed?"
if grep -q "FileChanged.*\.vrift\|path.*\.vrift" "$DAEMON_LOG" 2>/dev/null; then
    echo -e "   ${RED}❌ BUG: .vrift files being processed (should be ignored!)${NC}"
    grep "\.vrift" "$DAEMON_LOG" | grep -v "manifest\|Initialized" | head -3 | sed 's/^/      /'
    VRIFT_PROCESSED=1
else
    echo -e "   ${GREEN}✓ .vrift files not processed${NC}"
    VRIFT_PROCESSED=0
fi

# Check 3: Are src files processed correctly?
echo ""
echo "🔍 Check 3: src/ files being processed?"
if grep -q "FileChanged.*src\|Compensation.*src\|path.*app\.c" "$DAEMON_LOG" 2>/dev/null; then
    echo -e "   ${GREEN}✓ src/ files processed correctly${NC}"
    SRC_PROCESSED=1
else
    echo -e "   ${YELLOW}⚠ src/ files NOT processed (separate issue)${NC}"
    SRC_PROCESSED=0
fi

# Summary
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 RESULT"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [ $VRIFT_IN_ERRORS -eq 1 ] || [ $VRIFT_PROCESSED -eq 1 ]; then
    echo ""
    echo -e "${RED}❌ BUG #2 CONFIRMED: .vrift directory is NOT properly ignored${NC}"
    echo ""
    echo "Expected: .vrift/* files should NEVER appear in watch events"
    echo "Actual:   .vrift/* files are being watched/processed"
    echo ""
    echo "Evidence log: $DAEMON_LOG"
    echo ""
    echo "Root cause: IgnoreMatcher in watch.rs does not include .vrift pattern"
    exit 1
else
    echo ""
    echo -e "${GREEN}✅ PASS: .vrift directory is properly ignored${NC}"
    exit 0
fi
