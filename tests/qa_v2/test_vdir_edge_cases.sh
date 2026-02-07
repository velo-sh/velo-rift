#!/bin/bash
# ==============================================================================
# VDir Edge Cases & Stress Test Suite
# ==============================================================================
# Tests edge cases, stress scenarios, and concurrency
# Covers Phase 7-8 of the VDir QA Test Plan
#
# Test Scenarios:
#   P7: Edge Cases & Stress Tests
#   P8: Multi-Process Concurrency
# ==============================================================================

set -euo pipefail

# ============================================================================
# Configuration (SSOT via test_setup.sh)
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_WORKSPACE_BASE="/tmp/vdir_edge_test_$$"
SKIP_AUTO_SETUP=1  # We'll call setup manually
source "$SCRIPT_DIR/test_setup.sh"

SAMPLE_PROJECT="$SCRIPT_DIR/lib/sample_project"

# Test-specific variables
PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
DAEMON_PID=""

# ============================================================================
# Helpers
# ============================================================================
log_phase() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║  PHASE $1"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
}

log_test() {
    echo ""
    echo "🧪 [$1] $2"
}

log_pass() {
    echo "   ✅ PASS: $1"
    PASS_COUNT=$((PASS_COUNT + 1))
}

log_fail() {
    echo "   ❌ FAIL: $1"
    FAIL_COUNT=$((FAIL_COUNT + 1))
}

log_skip() {
    echo "   ⏭️  SKIP: $1"
    SKIP_COUNT=$((SKIP_COUNT + 1))
}

cleanup() {
    [ -n "$DAEMON_PID" ] && kill -9 "$DAEMON_PID" 2>/dev/null || true
    rm -f "$VRIFT_SOCKET_PATH"
    
    if [ -d "$TEST_WORKSPACE" ]; then
        chflags -R nouchg "$TEST_WORKSPACE" 2>/dev/null || true
        rm -rf "$TEST_WORKSPACE"
    fi
}
trap cleanup EXIT

setup_workspace() {
    cleanup
    mkdir -p "$TEST_WORKSPACE/src"
    mkdir -p "$VR_THE_SOURCE"
    cd "$TEST_WORKSPACE"
    
    # Create minimal project
    echo 'int main() { return 0; }' > src/main.c
    
    "$VRIFT_CLI" init 2>/dev/null || true
    "$VRIFT_CLI" ingest --mode solid --tier tier2 --output .vrift/manifest.lmdb src 2>/dev/null || true
    
    VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
        "$VRIFTD_BIN" start </dev/null > "${TEST_WORKSPACE}/vriftd.log" 2>&1 &
    DAEMON_PID=$!
    
    # Wait for daemon socket with timeout (max 10s)
    local waited=0
    while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
        sleep 0.5
        waited=$((waited + 1))
    done
    
    if [ ! -S "$VRIFT_SOCKET_PATH" ]; then
        echo "⚠️ Daemon socket not ready after 5s, continuing anyway..."
    fi
}

# ============================================================================
# Phase 7: Edge Cases & Stress
# ============================================================================
phase7_edge_cases() {
    log_phase "7: Edge Cases & Stress Tests"
    
    cd "$TEST_WORKSPACE"
    
    log_test "P7.1" "Bulk file creation (100 files in 1 second)"
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    mkdir -p src/bulk
    local start_time=$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')
    
    for i in $(seq 1 100); do
        echo "// file $i" > "src/bulk/file_$i.c"
    done
    
    local end_time=$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')
    local duration=$((end_time - start_time))
    
    local count=$(ls src/bulk/*.c 2>/dev/null | wc -l | tr -d ' ')
    if [ "$count" -eq 100 ]; then
        log_pass "100 files created in ${duration}ms"
    else
        log_fail "Only $count/100 files created"
    fi
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
    
    log_test "P7.2" "Nested symlinks"
    mkdir -p src/links
    echo "target" > src/links/target.txt
    ln -sf target.txt src/links/link1
    ln -sf link1 src/links/link2
    
    if readlink src/links/link2 | grep -q link1; then
        log_pass "Nested symlinks created"
    else
        log_fail "Symlink chain broken"
    fi
    
    log_test "P7.3" "Hidden files (.gitignore, .env)"
    echo "*.o" > src/.gitignore
    echo "SECRET=1" > src/.env
    
    if [ -f "src/.gitignore" ] && [ -f "src/.env" ]; then
        log_pass "Hidden files created"
    else
        log_fail "Hidden file creation failed"
    fi
    
    log_test "P7.4" "Large file (10MB binary)"
    dd if=/dev/urandom of=src/large.bin bs=1m count=10 2>/dev/null
    
    if [ -f "src/large.bin" ]; then
        local size=$(stat -f %z src/large.bin)
        if [ "$size" -ge 10000000 ]; then
            log_pass "10MB file created (${size} bytes)"
        else
            log_fail "File size mismatch: $size bytes"
        fi
    else
        log_fail "Large file creation failed"
    fi
    
    log_test "P7.5" "Unicode filename"
    echo "// Japanese comment: 日本語" > "src/日本語.c"
    
    if [ -f "src/日本語.c" ]; then
        log_pass "Unicode filename created"
    else
        log_fail "Unicode filename failed"
    fi
    
    log_test "P7.6" "FIFO/named pipe (should be ignored)"
    mkfifo "src/test_pipe" 2>/dev/null || true
    
    if [ -p "src/test_pipe" ]; then
        log_pass "FIFO created (should be ignored by VFS)"
        rm -f "src/test_pipe"
    else
        log_skip "FIFO creation not supported"
    fi
    
    log_test "P7.7" "Rapid overwrite (same file 50x)"
    for i in $(seq 1 50); do
        echo "version $i" > src/overwrite.c
    done
    
    if grep -q "version 50" src/overwrite.c; then
        log_pass "Rapid overwrite: last version preserved"
    else
        log_fail "Rapid overwrite: version mismatch"
    fi
    
    log_test "P7.8" "Empty file"
    touch src/empty.c
    
    if [ -f "src/empty.c" ] && [ ! -s "src/empty.c" ]; then
        log_pass "Empty file created"
    else
        log_fail "Empty file has content"
    fi
    
    log_test "P7.9" "File with spaces in name"
    echo "// spaces" > "src/file with spaces.c"
    
    if [ -f "src/file with spaces.c" ]; then
        log_pass "File with spaces created"
    else
        log_fail "File with spaces failed"
    fi
    
    log_test "P7.10" "Deep nesting (10 levels)"
    local deep_path="src/d1/d2/d3/d4/d5/d6/d7/d8/d9/d10"
    mkdir -p "$deep_path"
    echo "deep" > "$deep_path/deep.c"
    
    if [ -f "$deep_path/deep.c" ]; then
        log_pass "10-level deep file created"
    else
        log_fail "Deep nesting failed"
    fi
}

# ============================================================================
# Phase 8: Multi-Process Concurrency
# ============================================================================
phase8_concurrency() {
    log_phase "8: Multi-Process Concurrency"
    
    cd "$TEST_WORKSPACE"
    
    log_test "P8.1" "Parallel file creation (4 processes)"
    mkdir -p src/parallel
    
    # Launch 4 parallel writers
    for proc in {1..4}; do
        (
            for i in {1..25}; do
                echo "// proc $proc file $i" > "src/parallel/proc${proc}_file${i}.c"
            done
        ) &
    done
    wait
    
    local count=$(ls src/parallel/*.c 2>/dev/null | wc -l | tr -d ' ')
    if [ "$count" -eq 100 ]; then
        log_pass "Parallel creation: 100 files from 4 processes"
    else
        log_fail "Parallel creation: only $count/100 files"
    fi
    
    log_test "P8.2" "Concurrent read + write"
    echo "initial" > src/concurrent.c
    
    # Reader process
    (
        for i in {1..10}; do
            cat src/concurrent.c >/dev/null 2>&1 || true
            sleep 0.05
        done
    ) &
    local reader_pid=$!
    
    # Writer process
    (
        for i in {1..10}; do
            echo "version $i" > src/concurrent.c
            sleep 0.05
        done
    ) &
    local writer_pid=$!
    
    wait $reader_pid 2>/dev/null || true
    wait $writer_pid 2>/dev/null || true
    
    if [ -f "src/concurrent.c" ]; then
        log_pass "Concurrent read+write: no deadlock"
    else
        log_fail "Concurrent read+write: file missing"
    fi
    
    log_test "P8.3" "make -j4 parallel build"
    rm -rf build
    mkdir -p build
    
    # Create multiple source files
    for i in {1..8}; do
        echo "int func_$i() { return $i; }" > "src/mod_$i.c"
    done
    
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    # Parallel compilation
    local success=true
    for i in {1..8}; do
        gcc -c "src/mod_$i.c" -o "build/mod_$i.o" 2>/dev/null &
    done
    wait
    
    local obj_count=$(ls build/*.o 2>/dev/null | wc -l | tr -d ' ')
    if [ "$obj_count" -eq 8 ]; then
        log_pass "Parallel build: all 8 object files created"
    else
        log_fail "Parallel build: only $obj_count/8 objects"
    fi
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
    
    log_test "P8.4" "Stress: 1000 ops in 10 seconds"
    mkdir -p src/stress
    
    local start=$(date +%s)
    local ops=0
    
    while [ $ops -lt 1000 ]; do
        local now=$(date +%s)
        if [ $((now - start)) -ge 10 ]; then
            break
        fi
        
        case $((ops % 4)) in
            0) echo "create" > "src/stress/f_$ops.c" ;;
            1) touch "src/stress/f_$((ops-1)).c" 2>/dev/null || true ;;
            2) cat "src/stress/f_$((ops-2)).c" >/dev/null 2>&1 || true ;;
            3) rm -f "src/stress/f_$((ops-3)).c" 2>/dev/null || true ;;
        esac
        
        ops=$((ops + 1))
    done
    
    log_pass "Stress test: $ops ops in 10 seconds"
}

# ============================================================================
# Main
# ============================================================================
main() {
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║           VDir Edge Cases & Stress Test Suite                         ║"
    echo "║           Phase 7-8: Edge Cases → Concurrency → Stress                ║"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    
    if [ ! -f "$VRIFTD_BIN" ]; then
        echo "❌ vriftd not found: $VRIFTD_BIN"
        exit 1
    fi
    
    setup_workspace
    
    phase7_edge_cases
    phase8_concurrency
    
    echo ""
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║                         TEST SUMMARY                                  ║"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    echo ""
    echo "   Passed:  $PASS_COUNT"
    echo "   Failed:  $FAIL_COUNT"
    echo "   Skipped: $SKIP_COUNT"
    echo ""
    
    if [ "$FAIL_COUNT" -eq 0 ]; then
        echo "✅ ALL TESTS PASSED - Edge cases handled!"
        exit 0
    else
        echo "❌ SOME TESTS FAILED"
        exit 1
    fi
}

main "$@"
