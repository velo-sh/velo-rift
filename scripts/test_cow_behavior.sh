#!/bin/bash
# ==============================================================================
# VFS COW (Copy-on-Write) Behavior Test
# ==============================================================================
#
# Tests Break-Before-Write behavior with shim injection.
# Uses DYLD_INSERT_LIBRARIES to inject the vrift shim.
#
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# Auto-detect target directory (prefer release)
if [ -d "$PROJECT_ROOT/target/release" ]; then
    TARGET_DIR="release"
else
    TARGET_DIR="debug"
fi

SHIM_PATH="$PROJECT_ROOT/target/$TARGET_DIR/libvrift_shim.dylib"
VRIFT_BIN="$PROJECT_ROOT/target/$TARGET_DIR/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/$TARGET_DIR/vriftd"

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

# Run command with shim injection
with_shim() {
    DYLD_INSERT_LIBRARIES="$SHIM_PATH" \
    DYLD_FORCE_FLAT_NAMESPACE=1 \
    VRIFT_INCEPTION=1 \
    VRIFT_PROJECT_ROOT="$TEST_DIR" \
    VRIFT_MANIFEST="$TEST_DIR/.vrift/manifest.lmdb" \
    VRIFT_VFS_PREFIX="$TEST_DIR" \
    "$@"
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

    # Fallback to common CAS locations (only the_source paths per RFC-0039)
    local roots=("$HOME/.vrift/the_source" "/tmp/vrift/the_source")
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
    log_header "Setup"
    
    # Build if needed
    if [[ ! -f "$SHIM_PATH" ]]; then
        echo "Building shim..."
        cargo build -p vrift-shim --quiet
    fi
    
    if [[ ! -f "$VRIFT_BIN" ]]; then
        echo "Building CLI..."
        cargo build -p vrift-cli --quiet
    fi
    
    # Kill any existing daemon
    pkill -f vriftd 2>/dev/null || true
    rm -f /tmp/vrift.sock
    sleep 1
    
    # Clean test environment (need to remove uchg first)
    export TEST_DIR="/tmp/vfs_cow_test"
    chflags -R nouchg "$TEST_DIR" ~/.vrift/the_source 2>/dev/null || true
    rm -rf "$TEST_DIR" ~/.vrift/the_source
    mkdir -p "$TEST_DIR/deps"
    cd "$TEST_DIR"
    
    # Create test file with known content
    echo "original content from CAS" > deps/test_file.txt
    
    echo "Before ingest:"
    ls -la deps/
    
    # Ingest into CAS with explicit root
    export VR_THE_SOURCE="$TEST_DIR/cas_root"
    mkdir -p "$VR_THE_SOURCE"
    "$VRIFT_BIN" ingest --mode solid --tier tier1 --output .vrift/manifest.lmdb deps 2>&1 | grep -E "Complete|files|blobs" || true
    
    echo ""
    echo "After ingest:"
    ls -la deps/
    
    # Start daemon for IPC
    echo ""
    echo "Starting daemon..."
    "$VRIFTD_BIN" start 2>&1 &
    DAEMON_PID=$!
    sleep 2
    
    if kill -0 $DAEMON_PID 2>/dev/null; then
        echo "Daemon started (PID: $DAEMON_PID)"
    else
        echo "WARNING: Daemon failed to start"
    fi
    
    # Record CAS state
    CAS_PATH=$(get_cas_path deps/test_file.txt)
    CAS_INODE=$(stat -f %i "$CAS_PATH" 2>/dev/null || ls -i "$CAS_PATH" 2>/dev/null | awk '{print $1}')
    CAS_CONTENT=$(cat "$CAS_PATH" 2>/dev/null)
    
    echo ""
    echo "CAS Info:"
    echo "  Path: $CAS_PATH"
    echo "  Inode: $CAS_INODE"
    echo "  Content: $CAS_CONTENT"
}

# ==============================================================================
# TEST 1: Read works (baseline)
# ==============================================================================

test_read_works() {
    log_header "TEST 1: Read from VFS file"
    
    cd "$TEST_DIR"
    
    # Test with shim
    local content=$(with_shim cat deps/test_file.txt)
    
    if [[ "$content" == "original content from CAS" ]]; then
        pass "Read returns correct content: '$content'"
    else
        fail "Read content" "original content from CAS" "$content"
    fi
}

# ==============================================================================
# TEST 2: Write triggers COW
# ==============================================================================

test_write_triggers_cow() {
    log_header "TEST 2: Write triggers COW"
    
    cd "$TEST_DIR"
    
    # Clean any previous temp files
    rm -f /tmp/vrift_cow_*.tmp
    
    echo "Attempting write via shim-injected process..."
    
    # Create a C program that opens file for write
    cat > /tmp/write_test.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc != 2) {
        fprintf(stderr, "Usage: %s <file>\n", argv[0]);
        return 1;
    }
    
    int fd = open(argv[1], O_WRONLY | O_TRUNC);
    if (fd < 0) {
        perror("open");
        return 1;
    }
    
    const char *msg = "MODIFIED BY COW TEST\n";
    if (write(fd, msg, strlen(msg)) < 0) {
        perror("write");
        close(fd);
        return 1;
    }
    
    close(fd);
    printf("Write successful\n");
    return 0;
}
EOF
    
    gcc -o /tmp/write_test /tmp/write_test.c
    
    # Run write program with shim
    if with_shim /tmp/write_test "$TEST_DIR/deps/test_file.txt" 2>&1; then
        # Check COW temp file exists and has modified content
        local temp_file=$(ls /tmp/vrift_cow_*.tmp 2>/dev/null | head -1)
        if [[ -n "$temp_file" ]]; then
            local temp_content=$(cat "$temp_file")
            if [[ "$temp_content" == "MODIFIED BY COW TEST" ]]; then
                pass "COW temp file created with modified content"
            else
                fail "COW temp content" "MODIFIED BY COW TEST" "$temp_content"
            fi
        else
            fail "COW temp file" "exists" "not found"
        fi
        
        # Check CAS is unchanged (critical Iron Law invariant!)
        local cas_after=$(cat "$CAS_PATH" 2>/dev/null)
        if [[ "$cas_after" == "$CAS_CONTENT" ]]; then
            pass "CAS unchanged after COW (Iron Law preserved): '$cas_after'"
        else
            fail "CAS immutable" "$CAS_CONTENT" "$cas_after"
        fi
    else
        fail "COW write" "success" "write failed"
    fi
}

# ==============================================================================
# TEST 3: Write without shim should fail (uchg protection)
# ==============================================================================

test_write_without_shim_fails() {
    log_header "TEST 3: Write without shim fails (uchg protection)"
    
    cd "$TEST_DIR"
    
    # Re-ingest to reset
    echo "resetting content" > /tmp/reset.txt
    cp /tmp/reset.txt deps/test_file.txt 2>/dev/null || true
    "$VRIFT_BIN" ingest deps 2>&1 | grep -E "Complete" || true
    
    # Try to write WITHOUT shim - should fail due to uchg
    if echo "should fail" > deps/test_file.txt 2>/dev/null; then
        fail "Write without shim" "EPERM (blocked)" "write succeeded"
    else
        pass "Write without shim blocked by uchg"
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
        echo "  ✅ ALL TESTS PASSED - COW behavior works correctly"
        echo "═══════════════════════════════════════════════════════════"
        exit 0
    else
        echo "═══════════════════════════════════════════════════════════"
        echo "  ❌ SOME TESTS FAILED"
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
    echo "║     VFS COW (Copy-on-Write) Test Suite                   ║"
    echo "╠══════════════════════════════════════════════════════════╣"
    echo "║  Tests shim-based Break-Before-Write behavior            ║"
    echo "║  - Read from VFS: works                                  ║"
    echo "║  - Write with shim: COW → local copy                     ║"
    echo "║  - Write without shim: blocked by uchg                   ║"
    echo "╚══════════════════════════════════════════════════════════╝"
    
    setup
    test_read_works
    test_write_triggers_cow
    test_write_without_shim_fails
    
    print_summary
}

main "$@"
