#!/bin/bash
# RFC-0047 P0 Test: unlink() Behavior
# Tests actual unlink behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== RFC-0047 P0: unlink() Behavior ==="
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test unlink with Python
python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
test_file = os.path.join(test_dir, "to_delete.txt")

try:
    # Create file
    with open(test_file, 'w') as f:
        f.write("File to delete")
    
    # Verify it exists
    if not os.path.exists(test_file):
        print("❌ FAIL: Could not create test file")
        sys.exit(1)
    
    # Unlink (delete)
    os.unlink(test_file)
    
    # Verify it's gone
    if os.path.exists(test_file):
        print("❌ FAIL: unlink did not delete file")
        sys.exit(1)
    
    print("✅ PASS: unlink works correctly")
    print("   File created and successfully deleted")
    sys.exit(0)
    
except PermissionError as e:
    print(f"❌ FAIL: Permission denied (EROFS?): {e}")
    sys.exit(1)
except Exception as e:
    print(f"unlink error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import sys
f = '$TEST_DIR/delete_me.txt'
open(f, 'w').write('test')
os.unlink(f)
if not os.path.exists(f):
    print('✅ PASS: unlink works')
    sys.exit(0)
sys.exit(1)
"
