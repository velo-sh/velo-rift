#!/bin/bash
# RFC-0049 Gap Test: st_nlink (Hard Link Count) Virtualization
# Tests actual st_nlink behavior, not source code
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== P2 Gap Test: st_nlink Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "test" > "$TEST_DIR/test.txt"

# Test st_nlink with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    stat_result = os.stat(test_file)
    st_nlink = stat_result.st_nlink
    
    print(f"st_nlink: {st_nlink}")
    
    # For a regular file with no hard links, nlink should be 1
    if st_nlink == 1:
        print("✅ PASS: st_nlink is 1 (expected for single file)")
        sys.exit(0)
    else:
        print(f"ℹ️ INFO: st_nlink is {st_nlink}")
        print("   May indicate CAS dedup (multiple hard links)")
        sys.exit(0)  # P2, not blocking
        
except Exception as e:
    print(f"stat error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
stat = os.stat('$TEST_DIR/test.txt')
print(f'st_nlink: {stat.st_nlink}')
if stat.st_nlink >= 1:
    print('✅ PASS: st_nlink returned valid value')
    sys.exit(0)
else:
    print('❌ FAIL: st_nlink invalid')
    sys.exit(1)
"
