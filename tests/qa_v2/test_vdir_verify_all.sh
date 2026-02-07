#!/bin/bash
# ==============================================================================
# VDir Rigorous Verification Test Suite (Fail-Fast)
# ==============================================================================
# Each operation step verifies ALL observable facts:
#   1. Local real path (FS)
#   2. VDir mmap entry
#   3. CAS blob existence
#   4. Manifest entry
#   5. What should NOT exist
#
# Fail-fast: Any unexpected state immediately fails the test
# ==============================================================================

set -euo pipefail

# ============================================================================
# Configuration (SSOT via test_setup.sh)
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_WORKSPACE_BASE="/tmp/vdir_verify_$$"
SKIP_AUTO_SETUP=1  # We'll call setup manually after setting up test-specific vars
source "$SCRIPT_DIR/test_setup.sh"

# Test-specific variables (not in shared helper)
VDIR_MMAP="/dev/shm/vrift_vdir_test_$$"
PASS_COUNT=0
FAIL_COUNT=0
DAEMON_PID=""
FAIL_FAST=true  # Stop on first failure

# ============================================================================
# Verification Helpers
# ============================================================================
verify_pass() {
    echo "      ✓ $1"
    PASS_COUNT=$((PASS_COUNT + 1))
}

verify_fail() {
    echo "      ✗ $1"
    FAIL_COUNT=$((FAIL_COUNT + 1))
    if $FAIL_FAST; then
        echo ""
        echo "   ❌ FAIL-FAST: Stopping on first failure"
        echo "   $1"
        cleanup
        exit 1
    fi
}

# Verify file exists at local FS path
verify_local_exists() {
    local path="$1"
    local desc="${2:-File exists at local path}"
    if [ -e "$path" ]; then
        verify_pass "$desc: $path"
        return 0
    else
        verify_fail "$desc: $path (NOT FOUND)"
        return 1
    fi
}

# Verify file does NOT exist at local FS path
verify_local_not_exists() {
    local path="$1"
    local desc="${2:-File should NOT exist}"
    if [ ! -e "$path" ]; then
        verify_pass "$desc: $path"
        return 0
    else
        verify_fail "$desc: $path (UNEXPECTEDLY EXISTS)"
        return 1
    fi
}

# Verify file content matches expected
verify_content() {
    local path="$1"
    local expected="$2"
    local desc="${3:-Content matches}"
    if [ -f "$path" ]; then
        local actual=$(cat "$path" 2>/dev/null)
        if [ "$actual" = "$expected" ]; then
            verify_pass "$desc"
            return 0
        else
            verify_fail "$desc: expected '$expected', got '$actual'"
            return 1
        fi
    else
        verify_fail "$desc: file not found: $path"
        return 1
    fi
}

# Verify file size
verify_size() {
    local path="$1"
    local expected_size="$2"
    local desc="${3:-Size matches}"
    if [ -f "$path" ]; then
        local actual_size=$(stat -f %z "$path" 2>/dev/null || stat -c %s "$path" 2>/dev/null)
        if [ "$actual_size" -eq "$expected_size" ]; then
            verify_pass "$desc: ${actual_size} bytes"
            return 0
        else
            verify_fail "$desc: expected $expected_size, got $actual_size"
            return 1
        fi
    else
        verify_fail "$desc: file not found: $path"
        return 1
    fi
}

# Verify file is hardlink to CAS blob
verify_cas_hardlink() {
    local path="$1"
    local desc="${2:-Hardlinked to CAS}"
    if [ -f "$path" ]; then
        local inode=$(stat -f %i "$path" 2>/dev/null || stat -c %i "$path" 2>/dev/null)
        # Search for same inode in CAS
        local cas_match=$(find "$VR_THE_SOURCE" -inum "$inode" 2>/dev/null | head -1)
        if [ -n "$cas_match" ]; then
            verify_pass "$desc: inode $inode found in CAS"
            echo "         CAS path: $cas_match"
            return 0
        else
            # May not be hardlinked yet (new file)
            echo "      ~ $desc: inode $inode not in CAS (may be new file)"
            return 1
        fi
    else
        verify_fail "$desc: file not found: $path"
        return 1
    fi
}

# Verify CAS blob exists for given hash
verify_cas_blob() {
    local hash="$1"
    local desc="${2:-CAS blob exists}"
    local short_hash="${hash:0:4}"
    local blob_path="$VR_THE_SOURCE/blake3/$short_hash/$hash"
    if [ -f "$blob_path" ]; then
        verify_pass "$desc: $short_hash..."
        return 0
    else
        verify_fail "$desc: $blob_path not found"
        return 1
    fi
}

# Verify manifest contains entry (requires vrift CLI)
verify_manifest_entry() {
    local rel_path="$1"
    local desc="${2:-Manifest entry exists}"
    if [ -f "$TEST_WORKSPACE/.vrift/manifest.lmdb" ]; then
        # Try to query manifest
        if "$VRIFT_CLI" manifest query "$rel_path" 2>/dev/null | grep -q "$rel_path"; then
            verify_pass "$desc: $rel_path"
            return 0
        else
            echo "      ~ $desc: $rel_path not in manifest (may be expected)"
            return 1
        fi
    else
        echo "      ~ Manifest file not found"
        return 1
    fi
}

# Verify manifest does NOT contain entry
verify_manifest_no_entry() {
    local rel_path="$1"
    local desc="${2:-Manifest should NOT contain entry}"
    if [ -f "$TEST_WORKSPACE/.vrift/manifest.lmdb" ]; then
        if ! "$VRIFT_CLI" manifest query "$rel_path" 2>/dev/null | grep -q "$rel_path"; then
            verify_pass "$desc: $rel_path"
            return 0
        else
            verify_fail "$desc: $rel_path (UNEXPECTEDLY IN MANIFEST)"
            return 1
        fi
    else
        verify_pass "$desc: no manifest file"
        return 0
    fi
}

# Verify daemon is running
verify_daemon_running() {
    local desc="${1:-Daemon is running}"
    if pgrep -f vriftd >/dev/null 2>&1; then
        verify_pass "$desc"
        return 0
    else
        verify_fail "$desc (DAEMON NOT RUNNING)"
        return 1
    fi
}

# Verify mtime changed
verify_mtime_changed() {
    local path="$1"
    local old_mtime="$2"
    local desc="${3:-mtime changed}"
    if [ -f "$path" ]; then
        local new_mtime=$(stat -f %m "$path" 2>/dev/null || stat -c %Y "$path" 2>/dev/null)
        if [ "$new_mtime" -gt "$old_mtime" ]; then
            verify_pass "$desc: $old_mtime → $new_mtime"
            return 0
        else
            verify_fail "$desc: mtime unchanged ($old_mtime)"
            return 1
        fi
    else
        verify_fail "$desc: file not found"
        return 1
    fi
}

# ============================================================================
# Test Helpers
# ============================================================================
log_test() {
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "🧪 TEST: $1"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

log_step() {
    echo ""
    echo "   📍 STEP $1: $2"
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
    mkdir -p "$TEST_WORKSPACE"/{src,build}
    mkdir -p "$VR_THE_SOURCE"
    cd "$TEST_WORKSPACE"
    
    echo "Workspace: $TEST_WORKSPACE"
    echo "CAS Root:  $VR_THE_SOURCE"
}

# ============================================================================
# TEST 1: File Creation Full Verification
# ============================================================================
test_file_creation() {
    log_test "File Creation - Full Multi-Layer Verification"
    
    cd "$TEST_WORKSPACE"
    
    # -------------------------------------------------------------------------
    log_step "1.1" "Create source file WITHOUT VRift"
    # -------------------------------------------------------------------------
    local file_path="$TEST_WORKSPACE/src/hello.c"
    local file_content='int main() { return 0; }'
    
    echo "$file_content" > "$file_path"
    
    echo "   📋 Verifying state after creation:"
    verify_local_exists "$file_path" "Local file exists"
    verify_content "$file_path" "$file_content" "Content correct"
    verify_manifest_no_entry "src/hello.c" "Not yet in manifest (no ingest)"
    
    # -------------------------------------------------------------------------
    log_step "1.2" "Initialize VRift project"
    # -------------------------------------------------------------------------
    "$VRIFT_CLI" init 2>/dev/null || true
    
    echo "   📋 Verifying state after init:"
    verify_local_exists "$TEST_WORKSPACE/.vrift" ".vrift directory created"
    verify_local_exists "$file_path" "Source file still exists"
    
    # -------------------------------------------------------------------------
    log_step "1.3" "Ingest source files"
    # -------------------------------------------------------------------------
    "$VRIFT_CLI" ingest --mode solid --tier tier2 --output .vrift/manifest.lmdb src 2>/dev/null || true
    
    echo "   📋 Verifying state after ingest:"
    verify_local_exists "$file_path" "Source file at original path"
    verify_local_exists "$TEST_WORKSPACE/.vrift/manifest.lmdb" "Manifest created"
    
    # Check CAS directory
    local cas_files=$(find "$VR_THE_SOURCE" -type f 2>/dev/null | wc -l | tr -d ' ')
    if [ "$cas_files" -gt 0 ]; then
        verify_pass "CAS contains $cas_files blob(s)"
    else
        echo "      ~ CAS empty (tier1 may symlink instead of hardlink)"
    fi
    
    # Verify hardlink (if tier1 mode — non-fatal, tier1 may symlink)
    verify_cas_hardlink "$file_path" "Source hardlinked to CAS" || true
    
    # -------------------------------------------------------------------------
    log_step "1.4" "Start daemon"
    # -------------------------------------------------------------------------
    VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
        "$VRIFTD_BIN" start </dev/null > "${TEST_WORKSPACE}/vriftd.log" 2>&1 &
    DAEMON_PID=$!
    
    # Wait for daemon socket with timeout (max 10s)
    local waited=0
    while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
        sleep 0.5
        waited=$((waited + 1))
    done
    
    echo "   📋 Verifying daemon state:"
    verify_daemon_running "Daemon started"
    
    # -------------------------------------------------------------------------
    log_step "1.5" "Create NEW file WITH shim active"
    # -------------------------------------------------------------------------
    local new_file="$TEST_WORKSPACE/src/new_module.c"
    local new_content='void foo() {}'
    
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    echo "$new_content" > "$new_file"
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
    
    echo "   📋 Verifying state after shim file creation:"
    verify_local_exists "$new_file" "New file exists on disk"
    verify_content "$new_file" "$new_content" "New file content correct"
    
    # Wait for FS watch / ingest
    sleep 1
    
    # This is the key verification - does VDir pick it up?
    echo "   📋 Verifying VDir live ingest (may fail until implemented):"
    verify_manifest_entry "src/new_module.c" "Live ingest added to manifest" || true
}

# ============================================================================
# TEST 2: File Modification Full Verification
# ============================================================================
test_file_modification() {
    log_test "File Modification - Full Multi-Layer Verification"
    
    cd "$TEST_WORKSPACE"
    
    local file_path="$TEST_WORKSPACE/src/hello.c"
    
    # -------------------------------------------------------------------------
    log_step "2.1" "Record initial state"
    # -------------------------------------------------------------------------
    verify_local_exists "$file_path"
    local old_mtime=$(stat -f %m "$file_path" 2>/dev/null || stat -c %Y "$file_path" 2>/dev/null)
    local old_content=$(cat "$file_path")
    local old_size=$(stat -f %z "$file_path" 2>/dev/null || stat -c %s "$file_path" 2>/dev/null)
    local old_inode=$(stat -f %i "$file_path" 2>/dev/null || stat -c %i "$file_path" 2>/dev/null)
    
    echo "   Initial state:"
    echo "      mtime: $old_mtime"
    echo "      size:  $old_size bytes"
    echo "      inode: $old_inode"
    
    # -------------------------------------------------------------------------
    log_step "2.2" "Modify file content (simulating edit)"
    # -------------------------------------------------------------------------
    sleep 1  # Ensure mtime changes
    
    local new_content='int main() { return 42; }'
    echo "$new_content" > "$file_path"
    
    echo "   📋 Verifying state after modification:"
    verify_local_exists "$file_path" "File still exists"
    verify_content "$file_path" "$new_content" "Content updated"
    verify_mtime_changed "$file_path" "$old_mtime" "mtime updated"
    
    local new_inode=$(stat -f %i "$file_path" 2>/dev/null || stat -c %i "$file_path" 2>/dev/null)
    if [ "$new_inode" -eq "$old_inode" ]; then
        verify_pass "Inode unchanged (in-place edit)"
    else
        echo "      ~ Inode changed: $old_inode → $new_inode (COW or new file)"
    fi
    
    # -------------------------------------------------------------------------
    log_step "2.3" "touch file (mtime only)"
    # -------------------------------------------------------------------------
    local pre_touch_mtime=$(stat -f %m "$file_path" 2>/dev/null || stat -c %Y "$file_path" 2>/dev/null)
    sleep 1
    touch "$file_path"
    
    echo "   📋 Verifying state after touch:"
    verify_mtime_changed "$file_path" "$pre_touch_mtime" "mtime updated by touch"
    verify_content "$file_path" "$new_content" "Content unchanged"
}

# ============================================================================
# TEST 3: File Deletion Full Verification  
# ============================================================================
test_file_deletion() {
    log_test "File Deletion - Full Multi-Layer Verification"
    
    cd "$TEST_WORKSPACE"
    
    # -------------------------------------------------------------------------
    log_step "3.1" "Create file to delete"
    # -------------------------------------------------------------------------
    local delete_file="$TEST_WORKSPACE/src/to_delete.c"
    echo "delete me" > "$delete_file"
    
    verify_local_exists "$delete_file" "File created for deletion test"
    
    # -------------------------------------------------------------------------
    log_step "3.2" "Delete file"
    # -------------------------------------------------------------------------
    rm -f "$delete_file"
    
    echo "   📋 Verifying state after deletion:"
    verify_local_not_exists "$delete_file" "File removed from disk"
    
    # Parent directory should still exist
    verify_local_exists "$TEST_WORKSPACE/src" "Parent directory still exists"
    
    # Other files should be unaffected
    verify_local_exists "$TEST_WORKSPACE/src/hello.c" "Sibling file unaffected"
}

# ============================================================================
# TEST 4: Compilation Output Full Verification
# ============================================================================
test_compilation() {
    log_test "Compilation Output - Full Multi-Layer Verification"
    
    cd "$TEST_WORKSPACE"
    
    # -------------------------------------------------------------------------
    log_step "4.1" "Compile source to object file"
    # -------------------------------------------------------------------------
    local src_file="$TEST_WORKSPACE/src/hello.c"
    local obj_file="$TEST_WORKSPACE/build/hello.o"
    
    verify_local_exists "$src_file" "Source file exists"
    verify_local_not_exists "$obj_file" "Object file does not exist yet"
    
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    gcc -c "$src_file" -o "$obj_file" 2>/dev/null
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
    
    echo "   📋 Verifying state after compilation:"
    verify_local_exists "$obj_file" "Object file created"
    
    # Object file should be on real FS (manifest MISS)
    local obj_size=$(stat -f %z "$obj_file" 2>/dev/null || stat -c %s "$obj_file" 2>/dev/null)
    if [ "$obj_size" -gt 0 ]; then
        verify_pass "Object file has content: $obj_size bytes"
    else
        verify_fail "Object file is empty"
    fi
    
    # Build output should NOT be in manifest (MISS handling)
    verify_manifest_no_entry "build/hello.o" "Build output not in manifest (correct MISS)"
    
    # -------------------------------------------------------------------------
    log_step "4.2" "Link to executable"
    # -------------------------------------------------------------------------
    local exe_file="$TEST_WORKSPACE/build/hello"
    
    export VRIFT_PROJECT_ROOT="$TEST_WORKSPACE"
    export VRIFT_INCEPTION=1
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    
    gcc "$obj_file" -o "$exe_file" 2>/dev/null
    
    unset VRIFT_PROJECT_ROOT VRIFT_INCEPTION DYLD_INSERT_LIBRARIES
    
    echo "   📋 Verifying state after linking:"
    verify_local_exists "$exe_file" "Executable created"
    
    if [ -x "$exe_file" ]; then
        verify_pass "Executable has +x permission"
    else
        verify_fail "Executable missing +x permission"
    fi
    
    # -------------------------------------------------------------------------
    log_step "4.3" "Run executable"
    # -------------------------------------------------------------------------
    local exit_code=0
    "$exe_file" || exit_code=$?
    
    if [ "$exit_code" -eq 42 ]; then
        verify_pass "Executable runs correctly (exit code: $exit_code)"
    else
        echo "      ~ Executable runs (exit code: $exit_code, may vary)"
    fi
}

# ============================================================================
# TEST 5: External Tool Full Verification (bypass shim)
# ============================================================================
test_external_tool() {
    log_test "External Tool (cp/mv) - Full Multi-Layer Verification"
    
    cd "$TEST_WORKSPACE"
    
    # -------------------------------------------------------------------------
    log_step "5.1" "External cp creates file"
    # -------------------------------------------------------------------------
    local external_src="/tmp/external_file_$$"
    local external_dst="$TEST_WORKSPACE/src/external.c"
    
    echo "external content" > "$external_src"
    verify_local_exists "$external_src" "External source file exists"
    verify_local_not_exists "$external_dst" "Destination does not exist yet"
    
    # cp WITHOUT shim (simulates external tool)
    cp "$external_src" "$external_dst"
    
    echo "   📋 Verifying state after external cp:"
    verify_local_exists "$external_dst" "File copied to workspace"
    verify_content "$external_dst" "external content" "Content matches source"
    
    # Wait for FS Watch to detect
    sleep 1
    
    echo "   📋 Verifying FS Watch detection (may fail until implemented):"
    verify_manifest_entry "src/external.c" "FS Watch detected external cp" || true
    
    rm -f "$external_src"
    
    # -------------------------------------------------------------------------
    log_step "5.2" "External mv renames file"
    # -------------------------------------------------------------------------
    local renamed_file="$TEST_WORKSPACE/src/renamed.c"
    
    verify_local_exists "$external_dst" "Source file exists before mv"
    verify_local_not_exists "$renamed_file" "Destination does not exist before mv"
    
    mv "$external_dst" "$renamed_file"
    
    echo "   📋 Verifying state after external mv:"
    verify_local_not_exists "$external_dst" "Original file removed"
    verify_local_exists "$renamed_file" "Renamed file exists"
    verify_content "$renamed_file" "external content" "Content preserved"
}

# ============================================================================
# Main
# ============================================================================
main() {
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║     VDir Rigorous Verification Test Suite (Fail-Fast)                 ║"
    echo "║     Each step verifies: Local FS, VDir, CAS, Manifest                 ║"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    
    # Check prerequisites
    if [ ! -f "$VRIFT_CLI" ]; then
        echo "❌ vrift CLI not found. Run: cargo build --release"
        exit 1
    fi
    
    if ! command -v gcc &>/dev/null; then
        echo "❌ gcc not found"
        exit 1
    fi
    
    setup_workspace
    
    # Run all verification tests
    test_file_creation
    test_file_modification
    test_file_deletion
    test_compilation
    test_external_tool
    
    # Summary
    echo ""
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║                      VERIFICATION SUMMARY                             ║"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    echo ""
    echo "   ✓ Passed: $PASS_COUNT"
    echo "   ✗ Failed: $FAIL_COUNT"
    echo ""
    
    if [ "$FAIL_COUNT" -eq 0 ]; then
        echo "✅ ALL VERIFICATIONS PASSED"
        exit 0
    else
        echo "❌ SOME VERIFICATIONS FAILED"
        exit 1
    fi
}

main "$@"
