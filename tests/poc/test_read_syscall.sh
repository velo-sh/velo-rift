#!/bin/bash
# Test: read() Syscall Behavior
# Priority: P0

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that read() correctly retrieves content from VFS files

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== Test: read() Syscall Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ" > "$TEST_DIR/test.txt"

# Test with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    fd = os.open(test_file, os.O_RDONLY)
    
    # Read chunk 1
    chunk1 = os.read(fd, 10)
    if chunk1 != b"0123456789":
        print(f"❌ FAIL: Chunk 1 mismatch: {chunk1}")
        sys.exit(1)
        
    # Read chunk 2
    chunk2 = os.read(fd, 10)
    if chunk2 != b"ABCDEFGHIJ":
        print(f"❌ FAIL: Chunk 2 mismatch: {chunk2}")
        sys.exit(1)
        
    os.close(fd)
    print("✅ PASS: read() syscall verified for sequential access")
    sys.exit(0)
    
except Exception as e:
    print(f"read() test error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
fd = os.open('$TEST_DIR/test.txt', os.O_RDONLY)
if os.read(fd, 4) == b'0123':
    print('✅ PASS: read behavior verified')
    os.close(fd)
    sys.exit(0)
sys.exit(1)
"
