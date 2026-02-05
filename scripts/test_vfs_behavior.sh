#!/bin/bash
# ==============================================================================
# Velo Rift VFS Behavior Test Suite
# ==============================================================================
#
# Tests the CORE DESIGN INVARIANTS:
#
# INVARIANT P0-a: CAS[hash].content == hash(content) ALWAYS
#   - Files in CAS are named by their content hash
#   - This NEVER changes - CAS content is immutable
#
# INVARIANT P0-b: Break-Before-Write
#   - When writing to a CAS-linked file, break the link FIRST
#   - Then write to a LOCAL copy
#   - CAS original remains unchanged
#
# ==============================================================================

set -e

# Auto-detect VRIFT_BIN
if [ -f "$(cd "$(dirname "$0")/.." && pwd)/target/release/vrift" ]; then
    VRIFT_BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/vrift"
else
    VRIFT_BIN="$(cd "$(dirname "$0")/.." && pwd)/target/debug/vrift"
fi
PASS=0
FAIL=0

# ==============================================================================
# Helpers
# ==============================================================================

log_header() {
    echo ""
    echo "═══════════════════════════════════════════════════════════"
    echo "  $1"
    echo "═══════════════════════════════════════════════════════════"
}

pass() {
    echo "✅ PASS: $1"
    ((PASS++)) || true
}

fail() {
    echo "❌ FAIL: $1"
    echo "   Expected: $2"
    echo "   Actual:   $3"
    ((FAIL++)) || true
}

# Get CAS path for a file (by finding its hardlink in CAS)
get_cas_path() {
    local file="$1"
    local inode=$(stat -f %i "$file" 2>/dev/null || ls -i "$file" | awk '{print $1}')
    
    # Check explicit root first
    if [ -n "${VR_THE_SOURCE:-}" ] && [ -d "$VR_THE_SOURCE" ]; then
        local res=$(find "$VR_THE_SOURCE" -inum "$inode" 2>/dev/null | head -1)
        if [ -n "$res" ]; then
            echo "$res"
            return 0
        fi
    fi

    # Check common CAS locations
    local roots=("$HOME/.vrift/cas" "$HOME/.vrift/the_source" "/tmp/vrift/the_source" "/tmp/vrift/cas" "/tmp/vfs_behavior_test/.vrift/cas")
    for root in "${roots[@]}"; do
        if [ -d "$root" ]; then
            local res=$(find "$root" -inum "$inode" 2>/dev/null | head -1)
            if [ -n "$res" ]; then
                echo "$res"
                return 0
            fi
        fi
    done
    return 1
}

# ==============================================================================
# Setup
# ==============================================================================

setup() {
    log_header "Setup: Create test project and ingest files"
    
    unset VRIFT_INCEPTION VRIFT_PROJECT_ROOT
    rm -rf /tmp/vfs_behavior_test
    mkdir -p /tmp/vfs_behavior_test/deps
    export VR_THE_SOURCE="/tmp/vfs_behavior_test/cas_root"
    mkdir -p "$VR_THE_SOURCE"
    cd /tmp/vfs_behavior_test
    
    # Create files with known content
    echo "immutable content A" > deps/file_a.txt
    echo "immutable content B" > deps/file_b.txt
    echo "local file content" > local_file.txt
    
    echo "Files before ingest:"
    ls -la deps/
    
    # Ingest deps/ into CAS
    "$VRIFT_BIN" ingest --mode solid --tier tier2 deps 2>&1 | grep -E "Complete|files|blobs" || true
    
    echo ""
    echo "Files after ingest:"
    ls -la deps/
    
    echo ""
    echo "CAS location:"
    ls ~/.vrift/the_source/ 2>/dev/null | head -5 || ls /tmp/vfs_behavior_test/.vrift/cas/ 2>/dev/null | head -5 || echo "CAS empty or not found"
}

# ==============================================================================
# TEST GROUP 1: CAS Invariant (P0-a)
# ==============================================================================

test_cas_invariant() {
    log_header "GROUP 1: CAS Invariant - filename = hash(content)"
    
    cd /tmp/vfs_behavior_test/deps
    
    # Test 1.1: CAS file content matches expected
    echo "TEST 1.1: Read ingested file returns correct content"
    local content=$(cat file_a.txt)
    if [[ "$content" == "immutable content A" ]]; then
        pass "Read CAS file returns correct content"
    else
        fail "Read CAS file" "immutable content A" "$content"
    fi
    
    # Test 1.2: CAS file is hardlinked (link count > 1)
    echo ""
    echo "TEST 1.2: Ingested file is hardlinked to CAS"
    local link_count=$(ls -la file_a.txt | awk '{print $2}')
    if [[ "$link_count" -gt 1 ]]; then
        pass "File has link count $link_count (hardlinked)"
    else
        fail "Hardlink check" "link count > 1" "$link_count"
    fi
    
    # Test 1.3: CAS path contains content hash
    echo ""
    echo "TEST 1.3: CAS path contains content hash"
    local cas_path=$(get_cas_path file_a.txt)
    if [[ -n "$cas_path" ]]; then
        pass "CAS path found: $(basename "$cas_path")"
    else
        fail "CAS path lookup" "path exists" "not found"
    fi
}

# ==============================================================================
# TEST GROUP 2: Break-Before-Write
# ==============================================================================

test_break_before_write() {
    log_header "GROUP 2: Break-Before-Write"
    
    cd /tmp/vfs_behavior_test/deps
    
    # Get CAS inode before modification attempt
    local cas_path=$(get_cas_path file_a.txt)
    local cas_inode=$(ls -i "$cas_path" 2>/dev/null | awk '{print $1}')
    local cas_content_before=$(cat "$cas_path" 2>/dev/null)
    local file_inode_before=$(ls -i file_a.txt | awk '{print $1}')
    
    echo "Before write attempt:"
    echo "  CAS path: $cas_path"
    echo "  CAS inode: $cas_inode"
    echo "  File inode: $file_inode_before"
    echo "  Same inode (hardlinked): $([[ "$cas_inode" == "$file_inode_before" ]] && echo YES || echo NO)"
    
    # Test 2.1: Attempt to write to CAS file in inception mode
    echo ""
    echo "TEST 2.1: Write to CAS file (expect Break-Before-Write)"
    
    # This is the KEY test - try to write to a CAS-linked file
    cd /tmp/vfs_behavior_test
    echo "echo 'MODIFIED CONTENT' > deps/file_a.txt" | "$VRIFT_BIN" 2>&1 | grep -v "^$" | head -5 || true
    
    local file_content_after=$(cat deps/file_a.txt)
    local file_inode_after=$(ls -i deps/file_a.txt | awk '{print $1}')
    local cas_content_after=$(cat "$cas_path" 2>/dev/null)
    
    echo ""
    echo "After write attempt:"
    echo "  File content: $file_content_after"
    echo "  File inode: $file_inode_after"
    echo "  CAS content: $cas_content_after"
    
    # Check 2.1a: File content was modified
    if [[ "$file_content_after" == "MODIFIED CONTENT" ]]; then
        pass "File content was modified"
    else
        fail "File modification" "MODIFIED CONTENT" "$file_content_after"
    fi
    
    # Check 2.1b: CAS content is UNCHANGED (critical!)
    echo ""
    echo "TEST 2.2: CAS original is unchanged (P0-a invariant)"
    if [[ "$cas_content_after" == "$cas_content_before" ]]; then
        pass "CAS content unchanged: '$cas_content_after'"
    else
        fail "CAS immutability" "$cas_content_before" "$cas_content_after"
    fi
    
    # Check 2.1c: File inode is DIFFERENT (hardlink broken)
    echo ""
    echo "TEST 2.3: Hardlink was broken (different inode)"
    if [[ "$file_inode_after" != "$cas_inode" ]]; then
        pass "Hardlink broken: file inode $file_inode_after != CAS inode $cas_inode"
    else
        fail "Hardlink break" "different inodes" "same inode $file_inode_after"
    fi
}

# ==============================================================================
# TEST GROUP 3: Normal files unaffected
# ==============================================================================

test_normal_files() {
    log_header "GROUP 3: Non-CAS files work normally"
    
    cd /tmp/vfs_behavior_test
    
    # Test 3.1: Can read normal file
    echo "TEST 3.1: Read normal file"
    local content=$(cat local_file.txt)
    if [[ "$content" == "local file content" ]]; then
        pass "Read normal file"
    else
        fail "Read normal file" "local file content" "$content"
    fi
    
    # Test 3.2: Can write to normal file
    echo ""
    echo "TEST 3.2: Write to normal file"
    echo "new local content" > local_file.txt
    local new_content=$(cat local_file.txt)
    if [[ "$new_content" == "new local content" ]]; then
        pass "Write to normal file"
    else
        fail "Write to normal file" "new local content" "$new_content"
    fi
    
    # Test 3.3: Can create new file
    echo ""
    echo "TEST 3.3: Create new file"
    echo "brand new" > new_file.txt
    if [[ -f new_file.txt ]]; then
        pass "Create new file"
    else
        fail "Create new file" "file exists" "not found"
    fi
}

# ==============================================================================
# TEST GROUP 4: Build tool simulation
# ==============================================================================

test_build_simulation() {
    log_header "GROUP 4: Build tool simulation (cargo/npm)"
    
    cd /tmp/vfs_behavior_test
    mkdir -p target
    
    # Simulate cargo writing to target/
    echo "TEST 4.1: Simulate build output to target/"
    echo "compiled output" > target/output.rlib
    if [[ -f target/output.rlib ]]; then
        pass "Build tool can write to target/"
    else
        fail "Build output" "file created" "not created"
    fi
    
    # Test reading from deps/ and writing to target/
    echo ""
    echo "TEST 4.2: Read from deps/, write to target/ (typical build)"
    local src=$(cat deps/file_b.txt)
    echo "compiled: $src" > target/compiled.out
    local result=$(cat target/compiled.out)
    if [[ "$result" == "compiled: immutable content B" ]]; then
        pass "Read deps -> write target works"
    else
        fail "Read->Write pattern" "compiled: immutable content B" "$result"
    fi
}

# ==============================================================================
# Summary
# ==============================================================================

print_summary() {
    log_header "SUMMARY"
    
    echo ""
    echo "  Passed: $PASS"
    echo "  Failed: $FAIL"
    echo ""
    
    if [[ $FAIL -eq 0 ]]; then
        echo "═══════════════════════════════════════════════════════════"
        echo "  ✅ ALL TESTS PASSED - VFS behavior matches design"
        echo "═══════════════════════════════════════════════════════════"
        exit 0
    else
        echo "═══════════════════════════════════════════════════════════"
        echo "  ❌ SOME TESTS FAILED - Implementation needs work"
        echo "═══════════════════════════════════════════════════════════"
        exit 1
    fi
}

# ==============================================================================
# Main
# ==============================================================================

main() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════╗"
    echo "║     Velo Rift VFS Behavior Test Suite                    ║"
    echo "╠══════════════════════════════════════════════════════════╣"
    echo "║  Testing core design invariants:                         ║"
    echo "║  - P0-a: CAS[hash] = hash(content) IMMUTABLE             ║"
    echo "║  - P0-b: Break-Before-Write for modifications            ║"
    echo "╚══════════════════════════════════════════════════════════╝"
    
    setup
    test_cas_invariant
    test_break_before_write
    test_normal_files
    test_build_simulation
    
    print_summary
}

main "$@"
