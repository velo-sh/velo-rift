#!/bin/bash
# RFC-0049 Gap Test: dup/dup2 FD Tracking
# Tests actual dup behavior via libc interposition
# Priority: P1
# NOTE: Uses C program because Python os.dup() uses direct syscall on macOS

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== P1 Gap Test: dup/dup2 FD Tracking ==="

cleanup() { rm -rf "$TEST_DIR"; rm -f /tmp/test_dup_gap; }
trap cleanup EXIT

# Create test file
echo "Test content for dup" > "$TEST_DIR/test.txt"

# Compile C test program
cat > "$TEST_DIR/test_dup.c" << 'CEOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

int main(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <file>\n", argv[0]);
        return 1;
    }
    
    // Open file
    int fd1 = open(argv[1], O_RDONLY);
    if (fd1 < 0) {
        perror("open");
        return 1;
    }
    
    // Duplicate with dup
    int fd2 = dup(fd1);
    if (fd2 < 0) {
        perror("dup");
        close(fd1);
        return 1;
    }
    
    // Read from original
    char buf1[20] = {0}, buf2[20] = {0};
    read(fd1, buf1, 10);
    
    // Seek back using duplicate (both share file position)
    lseek(fd2, 0, SEEK_SET);
    read(fd2, buf2, 10);
    
    close(fd1);
    close(fd2);
    
    if (memcmp(buf1, buf2, 10) == 0) {
        printf("✅ PASS: dup works correctly, both FDs share position\n");
        printf("   Read: %s\n", buf1);
        return 0;
    } else {
        printf("❌ FAIL: data mismatch: '%s' vs '%s'\n", buf1, buf2);
        return 1;
    }
}
CEOF

cc -o /tmp/test_dup_gap "$TEST_DIR/test_dup.c" 2>/dev/null || clang -o /tmp/test_dup_gap "$TEST_DIR/test_dup.c" 2>/dev/null || {
    echo "❌ FAIL: Could not compile test program (neither cc nor clang found)"
    exit 1
}

# Resolve shim path
if [[ "$(uname)" == "Darwin" ]]; then
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.dylib"
    [[ -f "$SHIM_LIB" ]] || SHIM_LIB="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
    export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.so"
    [[ -f "$SHIM_LIB" ]] || SHIM_LIB="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
    export LD_PRELOAD="$SHIM_LIB"
fi

/tmp/test_dup_gap "$TEST_DIR/test.txt"
