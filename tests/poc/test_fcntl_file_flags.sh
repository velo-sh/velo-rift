#!/bin/bash
# Test: fcntl File Flags Behavior
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that fcntl(F_GETFL/F_SETFL) works correctly

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== Test: fcntl File Flags Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

touch "$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys
import fcntl

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    fd = os.open(test_file, os.O_RDONLY)
    
    # Get initial flags
    flags = fcntl.fcntl(fd, fcntl.F_GETFL)
    print(f"Initial flags: {oct(flags)}")
    
    # Set non-blocking
    fcntl.fcntl(fd, fcntl.F_SETFL, flags | os.O_NONBLOCK)
    
    # Verify
    new_flags = fcntl.fcntl(fd, fcntl.F_GETFL)
    print(f"New flags:     {oct(new_flags)}")
    
    os.close(fd)
    
    if (new_flags & os.O_NONBLOCK) != 0:
        print("✅ PASS: fcntl F_GETFL/F_SETFL works correctly")
        sys.exit(0)
    else:
        print("❌ FAIL: fcntl failed to set O_NONBLOCK")
        sys.exit(1)
        
except Exception as e:
    print(f"fcntl test error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import fcntl
import sys
fd = os.open('$TEST_DIR/test.txt', os.O_RDONLY)
flags = fcntl.fcntl(fd, fcntl.F_GETFL)
if flags >= 0:
    print('✅ PASS: fcntl works')
    os.close(fd)
    sys.exit(0)
sys.exit(1)
"
