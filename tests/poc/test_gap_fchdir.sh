#!/bin/bash
# RFC-0049 Gap Test: fchdir() Behavior
# Tests actual fchdir behavior, not source code
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== P1 Gap Test: fchdir() Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test directory
mkdir -p "$TEST_DIR/subdir"
echo "file in subdir" > "$TEST_DIR/subdir/test.txt"

# Test fchdir with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
subdir = os.path.join(test_dir, "subdir")
original_cwd = os.getcwd()

try:
    # Open directory
    dir_fd = os.open(subdir, os.O_RDONLY | os.O_DIRECTORY)
    
    # Change to directory using fd
    os.fchdir(dir_fd)
    
    new_cwd = os.getcwd()
    print(f"fchdir result: {new_cwd}")
    
    # Verify we're in the subdir
    if new_cwd.endswith("subdir"):
        # Try to read file in current directory
        if os.path.exists("test.txt"):
            print("✅ PASS: fchdir works correctly")
            os.close(dir_fd)
            os.chdir(original_cwd)
            sys.exit(0)
    
    print(f"❌ FAIL: fchdir didn't change to expected directory")
    os.close(dir_fd)
    os.chdir(original_cwd)
    sys.exit(1)
    
except Exception as e:
    print(f"fchdir error: {e}")
    try:
        os.chdir(original_cwd)
    except:
        pass
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
original = os.getcwd()
subdir = '$TEST_DIR/subdir'
fd = os.open(subdir, os.O_RDONLY | os.O_DIRECTORY)
os.fchdir(fd)
if os.getcwd().endswith('subdir'):
    print('✅ PASS: fchdir works')
    os.close(fd)
    os.chdir(original)
    sys.exit(0)
os.close(fd)
os.chdir(original)
sys.exit(1)
"
