#!/bin/bash
# RFC-0047 P0 Test: rmdir() Behavior
# Tests actual rmdir behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== RFC-0047 P0: rmdir() Behavior ==="
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test rmdir with Python
python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
subdir = os.path.join(test_dir, "to_remove")

try:
    # Create directory
    os.makedirs(subdir)
    
    # Verify it exists
    if not os.path.isdir(subdir):
        print("❌ FAIL: Could not create test directory")
        sys.exit(1)
    
    # rmdir (should work on empty directory)
    os.rmdir(subdir)
    
    # Verify it's gone
    if os.path.exists(subdir):
        print("❌ FAIL: rmdir did not remove directory")
        sys.exit(1)
    
    print("✅ PASS: rmdir works correctly")
    print("   Directory created and successfully removed")
    sys.exit(0)
    
except OSError as e:
    if e.errno == 30:  # EROFS
        print(f"❌ FAIL: Read-only filesystem error: {e}")
    else:
        print(f"rmdir error: {e}")
    sys.exit(1)
except Exception as e:
    print(f"rmdir error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import sys
d = '$TEST_DIR/remove_me'
os.makedirs(d)
os.rmdir(d)
if not os.path.exists(d):
    print('✅ PASS: rmdir works')
    sys.exit(0)
sys.exit(1)
"
