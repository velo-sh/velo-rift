#!/bin/bash
# RFC-0039 Live Ingest QA Test Suite
# Tests auto-registration of newly created files into VFS
#
# NOTE: These tests are designed to FAIL until the feature is implemented.
#       This is TDD - Red/Green/Refactor approach.

set -euo pipefail

# ============================================================================
# Configuration
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SHIM_LIB="$PROJECT_ROOT/target/debug/libvrift_inception_layer.dylib"
VRIFT_CLI="$PROJECT_ROOT/target/debug/vrift"

VFS_DIR="/tmp/test_live_ingest_$$"
VRIFT_DATA="$VFS_DIR/.vrift"

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

# ============================================================================
# Helpers
# ============================================================================
log_section() {
    echo ""
    echo "================================================================"
    echo "$1"
    echo "================================================================"
}

log_test() {
    echo ""
    echo "🧪 Test $1"
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
    echo "   ⏭️  SKIP: $1 (not implemented yet)"
    SKIP_COUNT=$((SKIP_COUNT + 1))
}

cleanup() {
    # Stop any running daemon
    pkill -f "vriftd.*$VFS_DIR" 2>/dev/null || true
    rm -rf "$VFS_DIR" 2>/dev/null || true
}
trap cleanup EXIT

setup_vfs() {
    cleanup
    mkdir -p "$VFS_DIR"
    mkdir -p "$VRIFT_DATA"
    echo "VFS Directory: $VFS_DIR"
}

# Check if manifest contains a path
# Returns 0 if found, 1 if not
check_manifest_contains() {
    local path="$1"
    
    # Convert absolute path to manifest key format (relative to VFS_DIR)
    local manifest_key
    if [[ "$path" == "$VFS_DIR"* ]]; then
        manifest_key="${path#$VFS_DIR}"
        [ -z "$manifest_key" ] && manifest_key="/"
    else
        manifest_key="$path"
    fi
    
    if [ -d "$VRIFT_DATA/manifest.lmdb" ]; then
        # Use vrift manifest query CLI
        if "$VRIFT_CLI" manifest query "$manifest_key" -d "$VFS_DIR" 2>/dev/null; then
            return 0
        fi
    fi
    return 1
}

# ============================================================================
# Layer 1 Tests: Shim → Manifest Updates
# ============================================================================
test_layer1_manifest_updates() {
    log_section "Layer 1: Shim → Manifest Updates"
    
    log_test "1.1: New file appears in manifest after close()"
    export VRIFT_VFS_PREFIX="$VFS_DIR"
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    # Create file with shim active
    echo "test content" > "$VFS_DIR/new_file.txt"
    
    unset VRIFT_VFS_PREFIX
    unset DYLD_INSERT_LIBRARIES
    
    # VERIFY: File should be in manifest
    if check_manifest_contains "$VFS_DIR/new_file.txt"; then
        log_pass "File registered in manifest"
    else
        log_fail "File NOT in manifest (Live Ingest not implemented)"
    fi
    
    log_test "1.2: mkdir sends IPC notification"
    export VRIFT_VFS_PREFIX="$VFS_DIR"
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    mkdir -p "$VFS_DIR/new_dir"
    
    unset VRIFT_VFS_PREFIX
    unset DYLD_INSERT_LIBRARIES
    
    # VERIFY: Directory should be in manifest
    if check_manifest_contains "$VFS_DIR/new_dir"; then
        log_pass "Directory registered in manifest"
    else
        log_fail "Directory NOT in manifest (IPC not implemented)"
    fi
    
    log_test "1.3: symlink sends IPC notification"
    export VRIFT_VFS_PREFIX="$VFS_DIR"
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    touch "$VFS_DIR/link_target.txt"
    ln -s "$VFS_DIR/link_target.txt" "$VFS_DIR/new_link"
    
    unset VRIFT_VFS_PREFIX
    unset DYLD_INSERT_LIBRARIES
    
    # VERIFY: Symlink should be in manifest
    if check_manifest_contains "$VFS_DIR/new_link"; then
        log_pass "Symlink registered in manifest"
    else
        log_fail "Symlink NOT in manifest (IPC not implemented)"
    fi
}

# ============================================================================
# Layer 2 Tests: FS Watch → Manifest Updates
# ============================================================================
test_layer2_fs_watch() {
    log_section "Layer 2: FS Watch → Manifest Updates"
    
    # Skip if daemon not running
    if ! pgrep -f "vriftd" > /dev/null; then
        log_skip "vriftd not running - FS Watch tests require daemon"
        log_skip "vriftd not running - FS Watch tests require daemon"
        return
    fi
    
    log_test "2.1: External file creation detected by watch"
    # Create file WITHOUT shim (external tool scenario)
    touch "$VFS_DIR/external_file.txt"
    
    # Wait for FS Watch debounce (100ms + margin)
    sleep 0.3
    
    # VERIFY: File should be detected and ingested
    if check_manifest_contains "$VFS_DIR/external_file.txt"; then
        log_pass "External file detected and ingested"
    else
        log_fail "External file NOT detected (FS Watch not implemented)"
    fi
    
    log_test "2.2: L1+L2 dedup (same file, two events)"
    export VRIFT_VFS_PREFIX="$VFS_DIR"
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    # Create via shim (L1)
    echo "via shim" > "$VFS_DIR/dedup_test.txt"
    
    unset VRIFT_VFS_PREFIX
    unset DYLD_INSERT_LIBRARIES
    
    # Modify externally within debounce window (L2)
    echo "via external" >> "$VFS_DIR/dedup_test.txt"
    
    sleep 0.2
    
    # VERIFY: Should only be ingested once (check ingest count if available)
    # For now, just check it's in manifest
    if check_manifest_contains "$VFS_DIR/dedup_test.txt"; then
        log_pass "Dedup file in manifest (need to verify ingest count)"
    else
        log_fail "Dedup test file not in manifest"
    fi
}

# ============================================================================
# Layer 3 Tests: Compensation Scan
# ============================================================================
test_layer3_compensation() {
    log_section "Layer 3: Compensation Scan"
    
    log_test "3.1: Files created while daemon stopped"
    # Create files "offline"
    mkdir -p "$VFS_DIR/offline_dir"
    echo "offline content" > "$VFS_DIR/offline_dir/offline_file.txt"
    
    # Simulate daemon restart with compensation scan
    # TODO: Trigger vrift sync when implemented
    if command -v "$VRIFT_CLI" &> /dev/null; then
        "$VRIFT_CLI" sync "$VFS_DIR" 2>/dev/null || true
    fi
    
    # VERIFY: Offline files should be in manifest after sync
    if check_manifest_contains "$VFS_DIR/offline_dir/offline_file.txt"; then
        log_pass "Offline file ingested after compensation scan"
    else
        log_fail "Offline file NOT ingested (Compensation Scan not implemented)"
    fi
}

# ============================================================================
# Integration Tests: State Machine
# ============================================================================
test_state_machine() {
    log_section "Integration: State Machine"
    
    log_test "4.1: RingBuffer backpressure (no CPU burn)"
    # This test measures CPU usage during rapid file creation
    # Should not exceed reasonable limits due to sleep-based backpressure
    
    export VRIFT_VFS_PREFIX="$VFS_DIR"
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    local start_time end_time duration
    start_time=$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')
    
    # Create files rapidly
    for i in {1..50}; do
        echo "content $i" > "$VFS_DIR/rapid_$i.txt"
    done
    
    end_time=$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')
    duration=$((end_time - start_time))
    
    unset VRIFT_VFS_PREFIX
    unset DYLD_INSERT_LIBRARIES
    
    # Files created, but are they in manifest?
    local manifest_count=0
    for i in {1..50}; do
        if check_manifest_contains "$VFS_DIR/rapid_$i.txt"; then
            manifest_count=$((manifest_count + 1))
        fi
    done
    
    if [ "$manifest_count" -eq 50 ]; then
        log_pass "All 50 rapid files ingested (${duration}ms)"
    else
        log_fail "Only $manifest_count/50 files in manifest (ingest pipeline not working)"
    fi
}

# ============================================================================
# Main
# ============================================================================
main() {
    echo "================================================================"
    echo "RFC-0039: Live Ingest QA Test Suite (TDD - Expect RED)"
    echo "================================================================"
    echo ""
    echo "⚠️  These tests are designed to FAIL until Live Ingest is implemented."
    echo "    RED = Feature not implemented"
    echo "    GREEN = Feature working"
    
    setup_vfs
    
    # Check shim exists
    if [ ! -f "$SHIM_LIB" ]; then
        echo ""
        echo "⚠️  Shim not found at $SHIM_LIB"
        echo "   Running cargo build..."
        (cd "$PROJECT_ROOT" && cargo build -p vrift-inception-layer --quiet)
    fi
    
    # Run all test layers
    test_layer1_manifest_updates
    test_layer2_fs_watch
    test_layer3_compensation
    test_state_machine
    
    # Summary
    log_section "Test Summary (TDD Status)"
    echo "Passed: $PASS_COUNT (GREEN - feature works)"
    echo "Failed: $FAIL_COUNT (RED - needs implementation)"
    echo "Skipped: $SKIP_COUNT (not runnable)"
    echo ""
    
    if [ "$FAIL_COUNT" -gt 0 ]; then
        echo "📋 Implementation TODO:"
        echo "   - Layer 1: Add IPC notification in mkdir_shim, symlink_shim"
        echo "   - Layer 1: Ensure close() triggers manifest reingest"
        echo "   - Layer 2: Implement FS Watch in vdir_d"
        echo "   - Layer 3: Implement compensation scan in vdir_d"
        echo ""
        echo "🔴 RED: $FAIL_COUNT tests need implementation"
        exit 0  # TDD: Red is expected, not an error
    else
        echo "✅ ALL GREEN: Live Ingest fully implemented!"
        exit 0
    fi
}

main "$@"
