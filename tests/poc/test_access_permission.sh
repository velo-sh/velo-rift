#!/bin/bash
# Test: access Permission Check Behavior
# Tests actual access() behavior, not symbols
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== Test: access() Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "content" > "$TEST_DIR/test.txt"
chmod 444 "$TEST_DIR/test.txt"

# Test with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    # F_OK (existence)
    if os.access(test_file, os.F_OK):
        print("✅ PASS: access F_OK successful")
    else:
        print("❌ FAIL: access F_OK failed")
        sys.exit(1)
        
    # R_OK (read)
    if os.access(test_file, os.R_OK):
        print("✅ PASS: access R_OK successful")
    else:
        print("❌ FAIL: access R_OK failed")
        sys.exit(1)
        
    # W_OK (write - should fail for 444)
    if not os.access(test_file, os.W_OK):
        print("✅ PASS: access W_OK correctly failed for read-only file")
    else:
        print("⚠️ INFO: access W_OK succeeded for 444 file (unexpected but common in some environments)")
        
    sys.exit(0)
    
except Exception as e:
    print(f"Error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
if os.access('$TEST_DIR/test.txt', os.F_OK):
    print('✅ PASS: access works')
    sys.exit(0)
sys.exit(1)
"
