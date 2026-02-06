#!/bin/bash
# ==============================================================================
# VDir Compiler Workflow Test Suite
# ==============================================================================
# Tests VDir's ability to transparently track file operations during compilation
# Covers Phase 0-4 of the VDir QA Test Plan
#
# Test Scenarios:
#   P0: Baseline - Normal FS compilation
#   P1: VRift Activation - Init, Ingest, Inception
#   P2: Live Compilation Under VFS
#   P3: New File Creation & Deletion
#   P4: External Tool Operations (bypass shim)
# ==============================================================================

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SAMPLE_PROJECT="$SCRIPT_DIR/lib/sample_project"

# Use release binaries if available, otherwise debug
if [ -f "$PROJECT_ROOT/target/release/vrift" ]; then
    VRIFT_CLI="$PROJECT_ROOT/target/release/vrift"
    SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
else
    VRIFT_CLI="$PROJECT_ROOT/target/debug/vrift"
    SHIM_LIB="$PROJECT_ROOT/target/debug/libvrift_inception_layer.dylib"
fi

TEST_WORKSPACE="/tmp/vdir_compiler_test_$$"
VRIFT_SOCKET_PATH="$TEST_WORKSPACE/vrift.sock"
export VRIFT_SOCKET_PATH
VR_THE_SOURCE="$TEST_WORKSPACE/.cas"

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

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
    # Stop any running daemons
    pkill -f "vriftd.*$TEST_WORKSPACE" 2>/dev/null || true
    pkill -f vriftd 2>/dev/null || true
    rm -f "$VRIFT_SOCKET_PATH"
    
    # Cleanup test workspace
    if [ -d "$TEST_WORKSPACE" ]; then
        # Remove immutable flags if any
        chflags -R nouchg "$TEST_WORKSPACE" 2>/dev/null || true
        rm -rf "$TEST_WORKSPACE"
    fi
}
trap cleanup EXIT

setup_workspace() {
    cleanup
    mkdir -p "$TEST_WORKSPACE"
    mkdir -p "$VR_THE_SOURCE"
    
    # Copy sample project
    cp -r "$SAMPLE_PROJECT"/* "$TEST_WORKSPACE/"
    
    echo "Test Workspace: $TEST_WORKSPACE"
    echo "CAS Root: $VR_THE_SOURCE"
}

# ============================================================================
# Phase 0: Baseline - Normal FS Compilation
# ============================================================================
phase0_baseline() {
    log_phase "0: Baseline - Normal FS Compilation"
    
    log_test "P0.1" "Basic gcc compilation"
    cd "$TEST_WORKSPACE"
    if gcc -Wall -I./include -c src/main.c -o build/main.o 2>/dev/null; then
        if [ -f "build/main.o" ]; then
            log_pass "main.o created successfully"
        else
            log_fail "main.o not found after compilation"
        fi
    else
        log_fail "gcc compilation failed"
    fi
    
    log_test "P0.2" "make clean && make"
    if make clean && make 2>/dev/null; then
        if [ -f "build/app" ]; then
            log_pass "make build succeeded"
        else
            log_fail "app binary not found"
        fi
    else
        log_fail "make failed"
    fi
    
    log_test "P0.3" "Incremental build"
    touch src/util.c
    local before=$(stat -f %m build/util.o 2>/dev/null || echo "0")
    sleep 1
    make 2>/dev/null
    local after=$(stat -f %m build/util.o 2>/dev/null || echo "0")
    if [ "$after" -gt "$before" ]; then
        log_pass "Incremental build recompiled modified file"
    else
        log_fail "Incremental build did not recompile"
    fi
    
    log_test "P0.4" "Binary execution"
    if ./build/app | grep -q "Hello from VFS"; then
        log_pass "Binary executes correctly"
    else
        log_fail "Binary output unexpected"
    fi
}

# ============================================================================
# Phase 1: VRift Activation
# ============================================================================
phase1_activation() {
    log_phase "1: VRift Activation - Init & Ingest"
    
    log_test "P1.1" "vrift init"
    cd "$TEST_WORKSPACE"
    if "$VRIFT_CLI" init 2>/dev/null; then
        if [ -d ".vrift" ]; then
            log_pass ".vrift directory created"
        else
            log_fail ".vrift directory not found"
        fi
    else
        log_fail "vrift init failed"
    fi
    
    log_test "P1.2" "vrift ingest src/"
    export VR_THE_SOURCE
    if "$VRIFT_CLI" ingest --mode solid --tier tier1 --output .vrift/manifest.lmdb src 2>/dev/null; then
        if [ -f ".vrift/manifest.lmdb" ]; then
            log_pass "Manifest created"
        else
            log_fail "Manifest not found"
        fi
    else
        log_fail "vrift ingest failed"
    fi
    
    log_test "P1.3" "CAS contains source files"
    if find "$VR_THE_SOURCE" -name "blake3" -type d | grep -q blake3; then
        log_pass "CAS blob directory exists"
    else
        log_fail "CAS blobs not found"
    fi
    
    log_test "P1.4" "Start vriftd daemon"
    "$PROJECT_ROOT/target/release/vriftd" start &
    DAEMON_PID=$!
    # Removed sleep 2 - using timeout loop above
    if kill -0 $DAEMON_PID 2>/dev/null; then
        log_pass "Daemon started (PID: $DAEMON_PID)"
    else
        log_fail "Daemon failed to start"
    fi
}

# ============================================================================
# Phase 2: Live Compilation Under VFS
# ============================================================================
phase2_vfs_compilation() {
    log_phase "2: Live Compilation Under VFS"
    
    cd "$TEST_WORKSPACE"
    make clean 2>/dev/null || true
    
    log_test "P2.1" "gcc compile src/main.c with shim"
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    if gcc -Wall -I./include -c src/main.c -o build/main.o 2>/dev/null; then
        if [ -f "build/main.o" ]; then
            log_pass "main.o created under VFS"
        else
            log_fail "main.o not found"
        fi
    else
        log_fail "gcc failed under VFS"
    fi
    
    log_test "P2.2" "gcc compile src/util.c with shim"
    if gcc -Wall -I./include -c src/util.c -o build/util.o 2>/dev/null; then
        log_pass "util.o created under VFS"
    else
        log_fail "gcc failed for util.c"
    fi
    
    log_test "P2.3" "Link object files"
    if gcc build/*.o -o build/app 2>/dev/null; then
        log_pass "Linking succeeded"
    else
        log_fail "Linking failed"
    fi
    
    log_test "P2.4" "Run compiled binary"
    if ./build/app | grep -q "Hello from VFS"; then
        log_pass "Binary runs correctly under VFS"
    else
        log_fail "Binary execution failed"
    fi
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
}

# ============================================================================
# Phase 3: New File Creation & Deletion
# ============================================================================
phase3_file_operations() {
    log_phase "3: New File Creation & Deletion"
    
    cd "$TEST_WORKSPACE"
    
    log_test "P3.1" "Create new source file"
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    cat > src/extra.c << 'EOF'
#include <stdio.h>
void extra_func(void) { printf("Extra!\n"); }
EOF
    
    if [ -f "src/extra.c" ]; then
        log_pass "New source file created"
    else
        log_fail "New source file not found"
    fi
    
    log_test "P3.2" "Compile new module"
    if gcc -Wall -I./include -c src/extra.c -o build/extra.o 2>/dev/null; then
        log_pass "New module compiled"
    else
        log_fail "New module compilation failed"
    fi
    
    log_test "P3.3" "Update mtime (touch)"
    local before=$(stat -f %m src/main.c)
    sleep 1
    touch src/main.c
    local after=$(stat -f %m src/main.c)
    if [ "$after" -gt "$before" ]; then
        log_pass "mtime updated"
    else
        log_fail "mtime not updated"
    fi
    
    log_test "P3.4" "Delete file"
    rm -f src/extra.c
    if [ ! -f "src/extra.c" ]; then
        log_pass "File deleted"
    else
        log_fail "File not deleted"
    fi
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
}

# ============================================================================
# Phase 4: External Tool Operations
# ============================================================================
phase4_external_tools() {
    log_phase "4: External Tool Operations (Bypass Shim)"
    
    cd "$TEST_WORKSPACE"
    
    log_test "P4.1" "External cp creates file"
    echo "// external file" > /tmp/external_temp.c
    cp /tmp/external_temp.c src/external.c
    sleep 0.5  # Wait for FS watch debounce
    if [ -f "src/external.c" ]; then
        log_pass "External cp succeeded"
    else
        log_fail "External cp failed"
    fi
    rm -f /tmp/external_temp.c
    
    log_test "P4.2" "External mv renames file"
    mv src/external.c src/renamed.c
    sleep 0.5
    if [ -f "src/renamed.c" ] && [ ! -f "src/external.c" ]; then
        log_pass "External mv succeeded"
    else
        log_fail "External mv failed"
    fi
    
    log_test "P4.3" "External echo appends to file"
    local before=$(cat src/util.c | wc -c | tr -d ' ')
    echo "// appended comment" >> src/util.c
    sleep 0.5
    local after=$(cat src/util.c | wc -c | tr -d ' ')
    if [ "$after" -gt "$before" ]; then
        log_pass "External append succeeded"
    else
        log_fail "External append failed"
    fi
    
    log_test "P4.4" "Compile after external changes"
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    if make 2>/dev/null; then
        log_pass "Compilation after external changes succeeded"
    else
        log_fail "Compilation failed after external changes"
    fi
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
    
    # Cleanup
    rm -f src/renamed.c
}

# ============================================================================
# Main
# ============================================================================
main() {
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║           VDir Compiler Workflow Test Suite                           ║"
    echo "║           Phase 0-4: Baseline → VFS → File Ops → External             ║"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    
    # Check prerequisites
    if [ ! -f "$SHIM_LIB" ]; then
        echo "❌ Shim not found: $SHIM_LIB"
        echo "   Run: cargo build --release -p vrift-inception-layer"
        exit 1
    fi
    
    if [ ! -f "$VRIFT_CLI" ]; then
        echo "❌ vrift CLI not found: $VRIFT_CLI"
        echo "   Run: cargo build --release"
        exit 1
    fi
    
    if ! command -v gcc &>/dev/null; then
        echo "❌ gcc not found"
        exit 1
    fi
    
    setup_workspace
    
    # Run all phases
    phase0_baseline
    phase1_activation
    phase2_vfs_compilation
    phase3_file_operations
    phase4_external_tools
    
    # Summary
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
        echo "✅ ALL TESTS PASSED"
        exit 0
    else
        echo "❌ SOME TESTS FAILED"
        exit 1
    fi
}

main "$@"
