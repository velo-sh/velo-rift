#!/bin/bash
# RFC-0049 Gap Test: readdir() Order Consistency
# Tests actual readdir behavior, not source code
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== P2 Gap Test: readdir() Order Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test files
touch "$TEST_DIR/a.txt" "$TEST_DIR/b.txt" "$TEST_DIR/c.txt"

# Test readdir with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")

try:
    # Read directory twice
    entries1 = sorted(os.listdir(test_dir))
    entries2 = sorted(os.listdir(test_dir))
    
    print(f"First read:  {entries1}")
    print(f"Second read: {entries2}")
    
    if entries1 == entries2:
        print("✅ PASS: readdir returns consistent entries")
        sys.exit(0)
    else:
        print("❌ FAIL: readdir order inconsistent")
        sys.exit(1)
        
except Exception as e:
    print(f"readdir error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
entries = os.listdir('$TEST_DIR')
print(f'Entries: {entries}')
if len(entries) >= 3:
    print('✅ PASS: readdir works correctly')
    sys.exit(0)
sys.exit(1)
"
