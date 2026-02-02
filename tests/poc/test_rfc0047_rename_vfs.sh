#!/bin/bash
# RFC-0047 P0 Test: rename() Behavior
# Tests actual rename behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== RFC-0047 P0: rename() Behavior ==="
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test rename with Python
python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
old_path = os.path.join(test_dir, "old_name.txt")
new_path = os.path.join(test_dir, "new_name.txt")

try:
    # Create file
    with open(old_path, 'w') as f:
        f.write("Content to rename")
    
    # Verify it exists
    if not os.path.exists(old_path):
        print("❌ FAIL: Could not create test file")
        sys.exit(1)
    
    # Rename
    os.rename(old_path, new_path)
    
    # Verify old is gone, new exists
    if os.path.exists(old_path):
        print("❌ FAIL: Old file still exists after rename")
        sys.exit(1)
    
    if not os.path.exists(new_path):
        print("❌ FAIL: New file does not exist after rename")
        sys.exit(1)
    
    # Verify content
    with open(new_path, 'r') as f:
        content = f.read()
        if "Content to rename" not in content:
            print(f"❌ FAIL: Content changed after rename: {content}")
            sys.exit(1)
    
    print("✅ PASS: rename works correctly")
    print(f"   Renamed: old_name.txt -> new_name.txt")
    sys.exit(0)
    
except PermissionError as e:
    print(f"❌ FAIL: Permission denied (EROFS?): {e}")
    sys.exit(1)
except Exception as e:
    print(f"rename error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import sys
old = '$TEST_DIR/a.txt'
new = '$TEST_DIR/b.txt'
open(old, 'w').write('test')
os.rename(old, new)
if not os.path.exists(old) and os.path.exists(new):
    print('✅ PASS: rename works')
    sys.exit(0)
sys.exit(1)
"
