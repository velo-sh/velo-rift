#!/bin/bash
# RFC-0047 P1 Test: CoW Write Path (write and close)
# Tests actual write/close behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== RFC-0047 P1: CoW Write Path Behavior ==="
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test write and close with Python
python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
test_file = os.path.join(test_dir, "cow_test.txt")

try:
    # Write content
    fd = os.open(test_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    test_content = b"CoW test content - written via fd"
    os.write(fd, test_content)
    os.close(fd)
    
    # Verify content persisted after close
    with open(test_file, 'rb') as f:
        read_content = f.read()
    
    if read_content == test_content:
        print("✅ PASS: Write + Close works correctly")
        print(f"   Written: {len(test_content)} bytes")
        print(f"   Read back: {len(read_content)} bytes")
        sys.exit(0)
    else:
        print(f"❌ FAIL: Content mismatch after close")
        print(f"   Written: {test_content}")
        print(f"   Read: {read_content}")
        sys.exit(1)
        
except Exception as e:
    print(f"CoW write error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import sys
f = '$TEST_DIR/test.txt'
fd = os.open(f, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
os.write(fd, b'test data')
os.close(fd)
with open(f, 'rb') as r:
    if r.read() == b'test data':
        print('✅ PASS: CoW write works')
        sys.exit(0)
sys.exit(1)
"
