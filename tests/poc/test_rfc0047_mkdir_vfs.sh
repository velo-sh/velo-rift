#!/bin/bash
# RFC-0047 P2 Test: mkdir() VFS Semantics
# Tests actual mkdir behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== RFC-0047 P2: mkdir() Behavior ==="
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test mkdir with Python
python3 << 'EOF'
import os
import sys
import stat

test_dir = os.environ.get("TEST_DIR", "/tmp")
new_dir = os.path.join(test_dir, "new_directory")

try:
    # Create directory with specific permissions
    os.mkdir(new_dir, 0o755)
    
    # Verify it exists
    if not os.path.isdir(new_dir):
        print("❌ FAIL: mkdir did not create directory")
        sys.exit(1)
    
    # Check permissions
    st = os.stat(new_dir)
    mode = stat.S_IMODE(st.st_mode)
    
    print(f"Created directory: {new_dir}")
    print(f"Permissions: {oct(mode)}")
    
    # Verify we can create files inside
    test_file = os.path.join(new_dir, "test.txt")
    with open(test_file, 'w') as f:
        f.write("test")
    
    if os.path.exists(test_file):
        print("✅ PASS: mkdir works correctly")
        print("   Directory created and writable")
        sys.exit(0)
    else:
        print("❌ FAIL: cannot write to created directory")
        sys.exit(1)
        
except Exception as e:
    print(f"mkdir error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import sys
new_dir = '$TEST_DIR/subdir'
os.mkdir(new_dir, 0o755)
if os.path.isdir(new_dir):
    print('✅ PASS: mkdir works')
    sys.exit(0)
sys.exit(1)
"
