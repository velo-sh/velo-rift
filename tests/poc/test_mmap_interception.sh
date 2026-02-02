#!/bin/bash
# Test: mmap Interception for Large Libraries
# Tests actual mmap behavior, not source code
# Goal: Verify if mmap works correctly

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== Test: mmap Behavior ==="
echo "Goal: mmap must work correctly for file content access"
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file with known content
echo "Hello mmap world! This is test content." > "$TEST_DIR/test.txt"

# Test mmap with Python
python3 << 'EOF'
import mmap
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
test_file = os.path.join(test_dir, "test.txt")

try:
    # Open file
    fd = os.open(test_file, os.O_RDONLY)
    
    # Get file size
    file_size = os.fstat(fd).st_size
    
    # Memory map the file
    mm = mmap.mmap(fd, file_size, access=mmap.ACCESS_READ)
    
    # Read content
    content = mm.read(20)
    print(f"mmap content: {content}")
    
    # Seek and read
    mm.seek(0)
    all_content = mm.read()
    
    mm.close()
    os.close(fd)
    
    if b"Hello mmap world" in all_content:
        print("✅ PASS: mmap works correctly")
        print(f"   Read {len(all_content)} bytes via mmap")
        sys.exit(0)
    else:
        print(f"❌ FAIL: mmap content mismatch: {all_content}")
        sys.exit(1)
        
except Exception as e:
    print(f"mmap error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import mmap
import os
import sys
fd = os.open('$TEST_DIR/test.txt', os.O_RDONLY)
size = os.fstat(fd).st_size
mm = mmap.mmap(fd, size, access=mmap.ACCESS_READ)
content = mm.read()
mm.close()
os.close(fd)
if b'Hello mmap' in content:
    print('✅ PASS: mmap works')
    sys.exit(0)
sys.exit(1)
"
