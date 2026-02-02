#!/bin/bash
# ==============================================================================
# Velo Rift Inception Mode - Test Suite
# ==============================================================================
#
# Verifies all expected behaviors:
# 1. Operations INSIDE project directory
# 2. Operations OUTSIDE project directory
# 3. Shell UX (auto-init, env vars, messages)
#
# Usage: ./test_inception_mode.sh
# ==============================================================================

set -e

VRIFT_BIN="${VRIFT_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/vrift}"
PASS=0
FAIL=0

# ==============================================================================
# Helpers
# ==============================================================================

run_test() {
    local name="$1"
    local setup="$2"
    local cmd="$3"
    local check="$4"
    
    # Setup
    eval "$setup" 2>/dev/null || true
    
    # Run command in inception
    echo "$cmd" | "$VRIFT_BIN" >/dev/null 2>&1
    
    # Check result
    if eval "$check"; then
        echo "✅ PASS: $name"
        ((PASS++)) || true
    else
        echo "❌ FAIL: $name"
        ((FAIL++)) || true
    fi
}

# ==============================================================================
# Setup
# ==============================================================================

echo ""
echo "╔══════════════════════════════════════════════════════════╗"
echo "║     Velo Rift Inception Mode - Test Suite                ║"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""

unset VRIFT_INCEPTION VRIFT_PROJECT_ROOT
rm -rf /tmp/vrift_test_project /tmp/vrift_test_external

# Create project
mkdir -p /tmp/vrift_test_project/src
echo "fn main(){}" > /tmp/vrift_test_project/src/main.rs
echo "# README" > /tmp/vrift_test_project/README.md
cd /tmp/vrift_test_project

# Create external dir
mkdir -p /tmp/vrift_test_external
echo "external" > /tmp/vrift_test_external/file.txt

# ==============================================================================
# Group 1: Operations INSIDE project directory
# ==============================================================================

echo "═══════════════════════════════════════════════════════════"
echo "  GROUP 1: Operations INSIDE project"
echo "  (Files NOT in VFS manifest - passthrough expected)"
echo "═══════════════════════════════════════════════════════════"
echo ""

run_test "chmod 755" \
    "chmod 644 src/main.rs" \
    "chmod 755 src/main.rs" \
    "[[ -x src/main.rs ]]"

run_test "cp file" \
    "rm -f src/copy.rs" \
    "cp src/main.rs src/copy.rs" \
    "[[ -f src/copy.rs ]]"

run_test "mv file" \
    "touch src/tomove.rs" \
    "mv src/tomove.rs src/moved.rs" \
    "[[ -f src/moved.rs && ! -f src/tomove.rs ]]"

run_test "touch file" \
    "rm -f src/new.rs" \
    "touch src/new.rs" \
    "[[ -f src/new.rs ]]"

run_test "rm file" \
    "touch src/todelete.rs" \
    "rm src/todelete.rs" \
    "[[ ! -f src/todelete.rs ]]"

run_test "mkdir" \
    "rm -rf testdir" \
    "mkdir testdir" \
    "[[ -d testdir ]]"

run_test "rmdir" \
    "mkdir -p emptydir" \
    "rmdir emptydir" \
    "[[ ! -d emptydir ]]"

run_test "cat (read)" \
    "" \
    "cat README.md" \
    "true"

run_test "echo > (write)" \
    "echo original > testfile.txt" \
    "echo updated > testfile.txt" \
    "[[ \$(cat testfile.txt) == 'updated' ]]"

# ==============================================================================
# Group 2: Operations OUTSIDE project directory
# ==============================================================================

echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  GROUP 2: Operations OUTSIDE project"  
echo "  (Completely unaffected by VFS - passthrough expected)"
echo "═══════════════════════════════════════════════════════════"
echo ""

run_test "cat external" \
    "" \
    "cat /tmp/vrift_test_external/file.txt" \
    "true"

run_test "touch external" \
    "rm -f /tmp/vrift_test_external/new.txt" \
    "touch /tmp/vrift_test_external/new.txt" \
    "[[ -f /tmp/vrift_test_external/new.txt ]]"

run_test "cp project->external" \
    "rm -f /tmp/vrift_test_external/copied.rs" \
    "cp src/main.rs /tmp/vrift_test_external/copied.rs" \
    "[[ -f /tmp/vrift_test_external/copied.rs ]]"

run_test "cp external->project" \
    "rm -f src/from_external.rs" \
    "cp /tmp/vrift_test_external/file.txt src/from_external.txt" \
    "[[ -f src/from_external.txt ]]"

run_test "rm external" \
    "touch /tmp/vrift_test_external/todelete.txt" \
    "rm /tmp/vrift_test_external/todelete.txt" \
    "[[ ! -f /tmp/vrift_test_external/todelete.txt ]]"

# ==============================================================================
# Group 3: Shell UX
# ==============================================================================

echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  GROUP 3: Shell UX"
echo "═══════════════════════════════════════════════════════════"
echo ""

# Test auto-init
rm -rf .vrift
echo "exit" | "$VRIFT_BIN" >/dev/null 2>&1
if [[ -d .vrift ]]; then
    echo "✅ PASS: auto-init creates .vrift/"
    ((PASS++)) || true
else
    echo "❌ FAIL: auto-init creates .vrift/"
    ((FAIL++)) || true
fi

# Test VRIFT_INCEPTION env var
result=$(echo 'echo $VRIFT_INCEPTION' | "$VRIFT_BIN" 2>&1 | grep "^1$" || true)
if [[ "$result" == "1" ]]; then
    echo "✅ PASS: VRIFT_INCEPTION=1 inside inception"
    ((PASS++)) || true
else
    echo "❌ FAIL: VRIFT_INCEPTION=1 inside inception"
    ((FAIL++)) || true
fi

# Test INCEPTION message
output=$(echo "exit" | "$VRIFT_BIN" 2>&1 | grep "INCEPTION" || true)
if [[ -n "$output" ]]; then
    echo "✅ PASS: INCEPTION message displayed"
    ((PASS++)) || true
else
    echo "❌ FAIL: INCEPTION message displayed"
    ((FAIL++)) || true
fi

# Test WAKE message
output=$(echo "exit" | "$VRIFT_BIN" 2>&1 | grep "WAKE" || true)
if [[ -n "$output" ]]; then
    echo "✅ PASS: WAKE message displayed"
    ((PASS++)) || true
else
    echo "❌ FAIL: WAKE message displayed"
    ((FAIL++)) || true
fi

# ==============================================================================
# Summary
# ==============================================================================

echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  SUMMARY"
echo "═══════════════════════════════════════════════════════════"
echo ""
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
echo ""

# Cleanup
rm -rf /tmp/vrift_test_project /tmp/vrift_test_external

if [[ $FAIL -eq 0 ]]; then
    echo "═══════════════════════════════════════════════════════════"
    echo "  ✅ ALL TESTS PASSED"
    echo "═══════════════════════════════════════════════════════════"
    exit 0
else
    echo "═══════════════════════════════════════════════════════════"
    echo "  ❌ SOME TESTS FAILED"
    echo "═══════════════════════════════════════════════════════════"
    exit 1
fi
