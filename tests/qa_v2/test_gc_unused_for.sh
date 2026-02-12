#!/bin/bash
# =============================================================================
# GC QA Integration Test: --unused-for time-based cleanup
# =============================================================================
# Tests the full GC flow including:
#   - Duration parsing via CLI args
#   - Stale/fresh blob classification by mtime
#   - Dry-run output correctness
#   - Actual deletion with --delete
#   - Empty directory cleanup after deletion
#   - macOS immutable flag (uchg) handling
#   - Edge: all fresh → nothing to clean
#   - Edge: all stale → everything cleaned
#   - Edge: empty CAS directory
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Find vrift binary
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
if [ ! -f "$VRIFT_BIN" ]; then
    VRIFT_BIN="$PROJECT_ROOT/target/debug/vrift"
fi
if [ ! -f "$VRIFT_BIN" ]; then
    echo "ERROR: vrift binary not found. Run 'cargo build' first."
    exit 1
fi

# Setup
TEST_DIR=$(mktemp -d /tmp/vrift_gc_test.XXXXXX)
CAS_DIR="$TEST_DIR/cas"
BLAKE3_DIR="$CAS_DIR/blake3"
PASS=0
FAIL=0
TOTAL=0

cleanup() {
    if [ "$(uname -s)" == "Darwin" ]; then
        chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
    fi
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

log_pass() {
    PASS=$((PASS + 1))
    TOTAL=$((TOTAL + 1))
    echo "  ✅ PASS: $1"
}

log_fail() {
    FAIL=$((FAIL + 1))
    TOTAL=$((TOTAL + 1))
    echo "  ❌ FAIL: $1"
}

# Helper: create a fake CAS blob with a specific mtime
# Usage: create_blob <hash_hex_64chars> <size_bytes> <age_hours>
create_blob() {
    local hash="$1"
    local size="$2"
    local age_hours="$3"
    local p1="${hash:0:2}"
    local p2="${hash:2:2}"
    local dir="$BLAKE3_DIR/$p1/$p2"
    mkdir -p "$dir"
    local file="$dir/${hash}_${size}.bin"
    dd if=/dev/zero bs=1 count="$size" of="$file" 2>/dev/null
    # Set mtime to age_hours ago
    if [ "$(uname -s)" == "Darwin" ]; then
        local ts=$(date -v-${age_hours}H "+%Y%m%d%H%M.%S")
        touch -t "$ts" "$file"
    else
        touch -d "-${age_hours} hours" "$file"
    fi
}

# Helper: reset CAS directory
reset_cas() {
    if [ "$(uname -s)" == "Darwin" ]; then
        chflags -R nouchg "$CAS_DIR" 2>/dev/null || true
    fi
    rm -rf "$CAS_DIR"
    mkdir -p "$BLAKE3_DIR"
}

echo "========================================"
echo " GC --unused-for Integration Tests"
echo "========================================"
echo " Binary: $VRIFT_BIN"
echo " Test dir: $TEST_DIR"
echo ""

# ─────────────────────────────────────────────────────────────────────────────
# Test 1: CLI help shows --unused-for with destructive warning
# ─────────────────────────────────────────────────────────────────────────────
echo "── Test 1: CLI help text ──"
HELP=$("$VRIFT_BIN" gc --help 2>&1)
if echo "$HELP" | grep -q "unused-for"; then
    log_pass "CLI help shows --unused-for flag"
else
    log_fail "CLI help missing --unused-for flag"
fi
if echo "$HELP" | grep -qi "destructive"; then
    log_pass "CLI help shows DESTRUCTIVE warning"
else
    log_fail "CLI help missing DESTRUCTIVE warning"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 2: Empty CAS — nothing to clean
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 2: Empty CAS ──"
reset_cas
OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h 2>&1)
if echo "$OUTPUT" | grep -qi "nothing to clean\|empty"; then
    log_pass "Empty CAS reported correctly"
else
    log_fail "Empty CAS not handled: $OUTPUT"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 3: All fresh blobs — nothing to clean
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 3: All fresh blobs ──"
reset_cas
create_blob "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" 100 1
create_blob "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" 200 2

OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h 2>&1)
if echo "$OUTPUT" | grep -q "Stale.*0 files"; then
    log_pass "Zero stale blobs detected"
else
    log_fail "Fresh blobs incorrectly classified: $OUTPUT"
fi
if echo "$OUTPUT" | grep -q "Fresh.*2 files"; then
    log_pass "2 fresh blobs counted"
else
    log_fail "Fresh blob count wrong: $OUTPUT"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 4: Mixed stale and fresh — dry run classification
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 4: Mixed stale/fresh — dry run ──"
reset_cas
# 2 old blobs (48h)
create_blob "1111111111111111111111111111111111111111111111111111111111111111" 1024 48
create_blob "2222222222222222222222222222222222222222222222222222222222222222" 2048 72
# 1 fresh blob (1h)
create_blob "3333333333333333333333333333333333333333333333333333333333333333" 512 1

OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h 2>&1)
if echo "$OUTPUT" | grep -q "Stale.*2 files"; then
    log_pass "2 stale blobs detected in dry run"
else
    log_fail "Stale count wrong: $OUTPUT"
fi
if echo "$OUTPUT" | grep -q "Fresh.*1 file"; then
    log_pass "1 fresh blob preserved in dry run"
else
    log_fail "Fresh count wrong: $OUTPUT"
fi
# Dry run should NOT delete anything
BLOB_COUNT=$(find "$BLAKE3_DIR" -type f | wc -l | tr -d ' ')
if [ "$BLOB_COUNT" -eq 3 ]; then
    log_pass "Dry run did not delete any files"
else
    log_fail "Dry run unexpectedly deleted files: $BLOB_COUNT remaining"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 5: Actual deletion with --delete --yes
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 5: Actual deletion ──"
# Reuse the same CAS from Test 4 (if still there) or recreate
reset_cas
create_blob "1111111111111111111111111111111111111111111111111111111111111111" 1024 48
create_blob "2222222222222222222222222222222222222222222222222222222222222222" 2048 72
create_blob "3333333333333333333333333333333333333333333333333333333333333333" 512 1

OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h --delete --yes 2>&1)
if echo "$OUTPUT" | grep -q "2 blobs deleted"; then
    log_pass "2 stale blobs deleted"
else
    log_fail "Deletion count wrong: $OUTPUT"
fi
# Verify only fresh blob remains
REMAINING=$(find "$BLAKE3_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
if [ "$REMAINING" -eq 1 ]; then
    log_pass "Only 1 fresh blob remains after deletion"
else
    log_fail "Expected 1 remaining file, got $REMAINING"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 6: Empty directory cleanup
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 6: Empty directory cleanup ──"
# After test 5, L1/L2 dirs of deleted blobs should be removed
EMPTY_DIRS=$(find "$BLAKE3_DIR" -type d -empty 2>/dev/null | wc -l | tr -d ' ')
if [ "$EMPTY_DIRS" -eq 0 ]; then
    log_pass "No empty directories remain after GC"
else
    log_fail "Empty directories remain: $EMPTY_DIRS"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 7: All stale — everything cleaned
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 7: All stale — complete cleanup ──"
reset_cas
create_blob "4444444444444444444444444444444444444444444444444444444444444444" 100 48
create_blob "5555555555555555555555555555555555555555555555555555555555555555" 200 96
create_blob "6666666666666666666666666666666666666666666666666666666666666666" 300 72

OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h --delete --yes 2>&1)
if echo "$OUTPUT" | grep -q "3 blobs deleted"; then
    log_pass "All 3 stale blobs deleted"
else
    log_fail "All-stale deletion failed: $OUTPUT"
fi
REMAINING=$(find "$BLAKE3_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
if [ "$REMAINING" -eq 0 ]; then
    log_pass "CAS completely empty after full cleanup"
else
    log_fail "Files remain: $REMAINING"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 8: Duration formats (compound durations)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 8: Duration formats ──"
reset_cas
create_blob "7777777777777777777777777777777777777777777777777777777777777777" 100 3

# 1h — should flag the 3h-old blob as stale
OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 1h 2>&1)
if echo "$OUTPUT" | grep -q "Stale.*1 file"; then
    log_pass "1h duration correctly classifies 3h-old blob as stale"
else
    log_fail "1h duration test failed: $OUTPUT"
fi

# 7d — should keep the 3h-old blob
OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 7d 2>&1)
if echo "$OUTPUT" | grep -q "Stale.*0 files"; then
    log_pass "7d duration correctly keeps 3h-old blob as fresh"
else
    log_fail "7d duration test failed: $OUTPUT"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 9: macOS immutable flag handling
# ─────────────────────────────────────────────────────────────────────────────
if [ "$(uname -s)" == "Darwin" ]; then
    echo ""
    echo "── Test 9: macOS immutable flag (uchg) handling ──"
    reset_cas
    create_blob "8888888888888888888888888888888888888888888888888888888888888888" 100 48

    # Set uchg flag
    BLOB_FILE=$(find "$BLAKE3_DIR" -type f | head -1)
    chflags uchg "$BLOB_FILE"

    OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h --delete --yes 2>&1)
    if echo "$OUTPUT" | grep -q "1 blobs deleted"; then
        log_pass "Deleted blob despite uchg flag"
    else
        log_fail "uchg handling failed: $OUTPUT"
    fi
    REMAINING=$(find "$BLAKE3_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
    if [ "$REMAINING" -eq 0 ]; then
        log_pass "Immutable blob successfully removed"
    else
        log_fail "Immutable blob still present"
    fi
else
    echo ""
    echo "── Test 9: Skipped (macOS-only) ──"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 10: Invalid duration rejected
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 10: Invalid duration rejected ──"
if ! "$VRIFT_BIN" gc --unused-for "0h" 2>&1; then
    log_pass "Zero duration rejected"
else
    log_fail "Zero duration was accepted"
fi
if ! "$VRIFT_BIN" gc --unused-for "5w" 2>&1; then
    log_pass "Invalid unit 'w' rejected"
else
    log_fail "Invalid unit 'w' was accepted"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Test 11: Second run after cleanup — nothing to clean
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── Test 11: Idempotent — second run finds nothing ──"
reset_cas
create_blob "9999999999999999999999999999999999999999999999999999999999999999" 100 48
"$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h --delete --yes >/dev/null 2>&1

OUTPUT=$("$VRIFT_BIN" --the-source-root "$CAS_DIR" gc --unused-for 24h 2>&1)
if echo "$OUTPUT" | grep -qi "nothing to clean\|0 files"; then
    log_pass "Second run correctly reports nothing to clean"
else
    log_fail "Second run output unexpected: $OUTPUT"
fi

# ─────────────────────────────────────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "========================================"
echo " Results: $PASS/$TOTAL passed, $FAIL failed"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
echo "🎉 All tests passed!"
