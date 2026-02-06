#!/bin/bash
# test_openat_faccessat.sh - Test directory-relative syscall behavior
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== Test: openat/faccessat/fstatat Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test directory and file
mkdir -p "$TEST_DIR/dir"
echo "data" > "$TEST_DIR/dir/data.txt"

export TEST_DIR="$TEST_DIR"

# Test with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
dir_path = os.path.join(test_dir, "dir")
filename = "data.txt"

try:
    dfd = os.open(dir_path, os.O_RDONLY | os.O_DIRECTORY)
    
    # openat
    fd = os.open(filename, os.O_RDONLY, dir_fd=dfd)
    data = os.read(fd, 4)
    os.close(fd)
    
    if data == b"data":
        print("✅ PASS: openat successful")
    else:
        print(f"❌ FAIL: openat returned {data}")
        sys.exit(1)
        
    # faccessat
    if os.access(filename, os.R_OK, dir_fd=dfd):
        print("✅ PASS: faccessat successful")
    else:
        print("❌ FAIL: faccessat failed")
        sys.exit(1)
        
    os.close(dfd)
    sys.exit(0)
    
except Exception as e:
    print(f"Error: {e}")
    sys.exit(1)
EOF
