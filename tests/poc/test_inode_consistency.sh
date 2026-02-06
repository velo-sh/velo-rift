#!/bin/bash
# Test: Inode Consistency Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that a file's inode remains constant while open

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== Test: Inode Consistency Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

echo "content" > "$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    # Get stat by path
    st_path = os.stat(test_file)
    ino1 = st_path.st_ino
    print(f"Path inode: {ino1}")
    
    # Get stat by FD
    fd = os.open(test_file, os.O_RDONLY)
    st_fd = os.fstat(fd)
    ino2 = st_fd.st_ino
    print(f"FD inode:   {ino2}")
    
    os.close(fd)
    
    if ino1 == ino2:
        print("✅ PASS: Inode is consistent between path-stat and fd-stat")
        sys.exit(0)
    else:
        print(f"❌ FAIL: Inode mismatch! Path: {ino1}, FD: {ino2}")
        sys.exit(1)
        
except Exception as e:
    print(f"Inode test error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
f = '$TEST_DIR/test.txt'
i1 = os.stat(f).st_ino
fd = os.open(f, os.O_RDONLY)
i2 = os.fstat(fd).st_ino
os.close(fd)
if i1 == i2:
    print('✅ PASS: inode consistency verified')
    sys.exit(0)
sys.exit(1)
"
