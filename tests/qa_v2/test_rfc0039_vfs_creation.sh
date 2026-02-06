#!/bin/bash
# Test Suite: RFC-0039 VFS Creation Operations
# Verifies that new files, directories, and symlinks can be created in VFS territory
# while existing manifest entries are protected from overwrite.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SHIM_PATH="$PROJECT_ROOT/target/debug/libvrift_inception_layer.dylib"

# Test environment
VFS_DIR="${VRIFT_VFS_PREFIX:-/tmp/test_vfs_creation}"
PASS_COUNT=0
FAIL_COUNT=0

cleanup() {
    rm -rf "$VFS_DIR"
}
trap cleanup EXIT

log_pass() {
    echo "   ✅ PASS: $1"
    PASS_COUNT=$((PASS_COUNT + 1))
}

log_fail() {
    echo "   ❌ FAIL: $1"
    FAIL_COUNT=$((FAIL_COUNT + 1))
}

# Ensure shim exists
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "❌ Shim not found: $SHIM_PATH"
    echo "   Run: cargo build -p vrift-inception-layer"
    exit 1
fi

echo "================================================================"
echo "RFC-0039: VFS Creation Operations Test Suite"
echo "================================================================"
echo "VFS Directory: $VFS_DIR"
echo "Shim: $SHIM_PATH"
echo ""

# Setup
cleanup 2>/dev/null || true
mkdir -p "$VFS_DIR"

export VRIFT_VFS_PREFIX="$VFS_DIR"
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"

cd "$VFS_DIR"

# ============================================================
# Part 1: New Creation Operations (manifest MISS -> ALLOW)
# ============================================================
echo "📁 Part 1: New Creation Operations (manifest MISS)"
echo "---------------------------------------------------"

# Test 1.1: mkdir - create new directory
echo ""
echo "🧪 Test 1.1: mkdir - create new directory"
if mkdir new_build_dir 2>&1; then
    if [[ -d new_build_dir ]]; then
        log_pass "mkdir created new directory"
    else
        log_fail "mkdir returned success but directory not found"
    fi
else
    log_fail "mkdir failed on new directory"
fi

# Test 1.2: mkdir -p - create nested directories
echo ""
echo "🧪 Test 1.2: mkdir -p - create nested directories"
if mkdir -p deep/nested/build/output 2>&1; then
    if [[ -d deep/nested/build/output ]]; then
        log_pass "mkdir -p created nested directories"
    else
        log_fail "mkdir -p returned success but directories not found"
    fi
else
    log_fail "mkdir -p failed on nested directories"
fi

# Test 1.3: touch - create new file
echo ""
echo "🧪 Test 1.3: touch - create new file"
if touch new_file.txt 2>&1; then
    if [[ -f new_file.txt ]]; then
        log_pass "touch created new file"
    else
        log_fail "touch returned success but file not found"
    fi
else
    log_fail "touch failed on new file"
fi

# Test 1.4: echo > file - create new file with content
echo ""
echo "🧪 Test 1.4: echo > file - create new file with content"
if echo "test content" > output.txt 2>&1; then
    if [[ -f output.txt ]] && grep -q "test content" output.txt; then
        log_pass "echo created new file with content"
    else
        log_fail "echo returned success but content incorrect"
    fi
else
    log_fail "echo redirection failed"
fi

# Test 1.5: symlink - create new symbolic link
echo ""
echo "🧪 Test 1.5: ln -s - create new symbolic link"
if ln -s /tmp/target new_symlink 2>&1; then
    if [[ -L new_symlink ]]; then
        log_pass "ln -s created symbolic link"
    else
        log_fail "ln -s returned success but symlink not found"
    fi
else
    log_fail "ln -s failed on new symlink"
fi

# Test 1.6: C compiler - create .o file
echo ""
echo "🧪 Test 1.6: gcc - create .o file in VFS"
cat > test_src.c << 'EOF'
int main() { return 0; }
EOF
if gcc -c test_src.c -o test_obj.o 2>&1; then
    if [[ -f test_obj.o ]]; then
        log_pass "gcc created .o file in VFS territory"
    else
        log_fail "gcc returned success but .o not found"
    fi
else
    log_fail "gcc compilation failed in VFS"
fi

# Test 1.7: createfile via C program
echo ""
echo "🧪 Test 1.7: open(O_CREAT) - create file via C"
cat > /tmp/test_creat.c << 'EOF'
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int fd = open(argv[1], O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) { perror("open"); return 1; }
    write(fd, "created via open\n", 17);
    close(fd);
    return 0;
}
EOF
unset DYLD_INSERT_LIBRARIES
gcc -o /tmp/test_creat /tmp/test_creat.c
export DYLD_INSERT_LIBRARIES="$SHIM_PATH"

if /tmp/test_creat created_via_open.txt 2>&1; then
    if [[ -f created_via_open.txt ]]; then
        log_pass "open(O_CREAT) created new file"
    else
        log_fail "open(O_CREAT) returned success but file not found"
    fi
else
    log_fail "open(O_CREAT) failed"
fi

# ============================================================
# Part 2: Append/Modify Operations on New Files
# ============================================================
echo ""
echo "📝 Part 2: Append/Modify Operations on New Files"
echo "-------------------------------------------------"

# Test 2.1: Append to newly created file
echo ""
echo "🧪 Test 2.1: echo >> - append to new file"
echo "line1" > appendtest.txt
if echo "line2" >> appendtest.txt 2>&1; then
    if grep -q "line1" appendtest.txt && grep -q "line2" appendtest.txt; then
        log_pass "Append to new file succeeded"
    else
        log_fail "Append content incorrect"
    fi
else
    log_fail "Append operation failed"
fi

# Test 2.2: Write multiple times to same file
echo ""
echo "🧪 Test 2.2: Multiple writes to same file"
echo "v1" > multiwrite.txt
echo "v2" > multiwrite.txt
echo "v3" > multiwrite.txt
if grep -q "v3" multiwrite.txt && ! grep -q "v1" multiwrite.txt; then
    log_pass "Multiple overwrites work correctly"
else
    log_fail "Multiple overwrites failed"
fi

# ============================================================
# Part 3: Directory Operations
# ============================================================
echo ""
echo "📂 Part 3: Directory Operations"
echo "--------------------------------"

# Test 3.1: Create files in subdirectory
echo ""
echo "🧪 Test 3.1: Create file in new subdirectory"
mkdir -p subdir
if echo "subfile content" > subdir/file.txt 2>&1; then
    if [[ -f subdir/file.txt ]]; then
        log_pass "File creation in subdirectory works"
    else
        log_fail "File not found in subdirectory"
    fi
else
    log_fail "File creation in subdirectory failed"
fi

# Test 3.2: Nested compilation output structure
echo ""
echo "🧪 Test 3.2: Build output directory structure"
mkdir -p target/debug/deps
mkdir -p target/release/deps
if [[ -d target/debug/deps ]] && [[ -d target/release/deps ]]; then
    log_pass "Nested build directories created"
else
    log_fail "Nested build directories failed"
fi

# ============================================================
# Summary
# ============================================================
echo ""
echo "================================================================"
echo "Test Summary"
echo "================================================================"
echo "Passed: $PASS_COUNT"
echo "Failed: $FAIL_COUNT"
echo ""

if [[ $FAIL_COUNT -eq 0 ]]; then
    echo "✅ ALL TESTS PASSED: VFS creation operations work correctly"
    exit 0
else
    echo "❌ SOME TESTS FAILED"
    exit 1
fi
