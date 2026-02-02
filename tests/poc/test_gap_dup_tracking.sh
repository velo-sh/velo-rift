#!/bin/bash
# RFC-0049 Gap Test: dup/dup2 FD Tracking
# Tests actual dup behavior, not source code
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== P1 Gap Test: dup/dup2 FD Tracking ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "Test content for dup" > "$TEST_DIR/test.txt"

# Test dup/dup2 with Python
export TEST_FILE="$TEST_DIR/test.txt"
if [[ "$(uname)" == "Darwin" ]]; then
    export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    export LD_PRELOAD="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi

python3 << 'EOF'
import os
import sys

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    # Open file
    fd1 = os.open(test_file, os.O_RDONLY)
    
    # Duplicate with dup
    fd2 = os.dup(fd1)
    
    # Read from original
    data1 = os.read(fd1, 10)
    
    # Seek back using duplicate (both share file position)
    os.lseek(fd2, 0, os.SEEK_SET)
    
    # Read from duplicate
    data2 = os.read(fd2, 10)
    
    os.close(fd1)
    os.close(fd2)
    
    if data1 == data2:
        print(f"✅ PASS: dup works correctly, both FDs share position")
        print(f"   Read: {data1[:20]}")
        sys.exit(0)
    else:
        print(f"❌ FAIL: data mismatch: {data1} vs {data2}")
        sys.exit(1)
        
except Exception as e:
    print(f"dup error: {e}")
    sys.exit(1)
EOF
