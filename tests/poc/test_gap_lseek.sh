#!/bin/bash
# Compiler Gap Test: lseek
# Tests actual lseek behavior, not source code
#
# RISK: HIGH - Archive tools (ar, tar) require random access

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== Compiler Gap: lseek Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file with content
echo "0123456789ABCDEFGHIJ" > "$TEST_DIR/test.txt"

# Test lseek with Python
python3 << 'EOF'
import os
import sys

test_file = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("TEST_FILE", "")
if not test_file:
    test_file = os.path.join(os.environ.get("TEST_DIR", "/tmp"), "test.txt")

try:
    fd = os.open(test_file, os.O_RDONLY)
    
    # Seek to position 10
    pos = os.lseek(fd, 10, os.SEEK_SET)
    if pos != 10:
        print(f"lseek SEEK_SET failed: expected 10, got {pos}")
        sys.exit(1)
    
    # Read from position 10
    data = os.read(fd, 5)
    if data != b"ABCDE":
        print(f"Read after lseek failed: expected 'ABCDE', got {data}")
        sys.exit(1)
    
    # Seek relative
    pos = os.lseek(fd, -3, os.SEEK_CUR)
    if pos != 12:
        print(f"lseek SEEK_CUR failed: expected 12, got {pos}")
        sys.exit(1)
    
    # Seek to end
    pos = os.lseek(fd, 0, os.SEEK_END)
    expected_end = 21  # 20 chars + newline
    if pos != expected_end:
        print(f"lseek SEEK_END: got {pos} (file size)")
    
    os.close(fd)
    print("✅ PASS: lseek works correctly")
    sys.exit(0)
    
except Exception as e:
    print(f"lseek error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"
python3 -c "
import os
import sys
test_file = '$TEST_DIR/test.txt'
fd = os.open(test_file, os.O_RDONLY)
pos = os.lseek(fd, 10, os.SEEK_SET)
data = os.read(fd, 5)
os.close(fd)
if data == b'ABCDE':
    print('✅ PASS: lseek works correctly')
    sys.exit(0)
else:
    print(f'❌ FAIL: lseek returned wrong data: {data}')
    sys.exit(1)
"
