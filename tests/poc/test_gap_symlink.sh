#!/bin/bash
# Compiler Gap Test: symlink
# Tests actual symlink behavior, not source code
#
# RISK: MEDIUM - Library versioning uses symlinks

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== Compiler Gap: symlink Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "target content" > "$TEST_DIR/target.txt"

# Test symlink with Python
python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
target = os.path.join(test_dir, "target.txt")
link = os.path.join(test_dir, "link.txt")

try:
    # Create symlink
    os.symlink(target, link)
    
    # Verify symlink exists
    if not os.path.islink(link):
        print("❌ FAIL: symlink not created")
        sys.exit(1)
    
    # Read through symlink
    with open(link, 'r') as f:
        content = f.read()
        if "target content" in content:
            print("✅ PASS: symlink works correctly")
            print(f"   Link: {link} -> {os.readlink(link)}")
            sys.exit(0)
        else:
            print(f"❌ FAIL: content mismatch: {content}")
            sys.exit(1)
            
except Exception as e:
    print(f"symlink error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import sys

test_dir = '$TEST_DIR'
target = os.path.join(test_dir, 'target.txt')
link = os.path.join(test_dir, 'symlink.txt')

os.symlink(target, link)

if os.path.islink(link):
    print('✅ PASS: symlink created successfully')
    print(f'   {link} -> {os.readlink(link)}')
    sys.exit(0)
else:
    print('❌ FAIL: symlink not created')
    sys.exit(1)
"
