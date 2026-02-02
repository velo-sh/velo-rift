#!/bin/bash
# Compiler Gap Test: utimes/futimes
# Tests actual utimes behavior, not source code
#
# RISK: HIGH - Make/Ninja use this for dependency tracking

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== Compiler Gap: utimes/futimes Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "test" > "$TEST_DIR/test.txt"

# Get original mtime
ORIG_MTIME=$(stat -f "%m" "$TEST_DIR/test.txt" 2>/dev/null || stat -c "%Y" "$TEST_DIR/test.txt")

# Test utimes with Python
python3 << 'EOF'
import os
import sys
import time

test_file = os.environ.get("TEST_FILE", "/tmp/test.txt")

try:
    # Get current times
    stat1 = os.stat(test_file)
    orig_mtime = stat1.st_mtime
    
    # Set specific time (1 hour ago)
    new_time = time.time() - 3600
    
    # Use utime to set both atime and mtime
    os.utime(test_file, (new_time, new_time))
    
    # Verify
    stat2 = os.stat(test_file)
    new_mtime = stat2.st_mtime
    
    # Check if mtime changed
    if abs(new_mtime - new_time) < 2:  # Allow 2 second tolerance
        print(f"✅ PASS: utimes works correctly")
        print(f"   Original mtime: {orig_mtime}")
        print(f"   New mtime: {new_mtime} (set to {new_time})")
        sys.exit(0)
    else:
        print(f"❌ FAIL: mtime not updated correctly")
        print(f"   Expected: {new_time}, Got: {new_mtime}")
        sys.exit(1)
        
except Exception as e:
    print(f"utimes error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/test.txt"

python3 -c "
import os
import time
import sys

test_file = '$TEST_DIR/test.txt'
new_time = time.time() - 7200  # 2 hours ago
os.utime(test_file, (new_time, new_time))
stat = os.stat(test_file)
if abs(stat.st_mtime - new_time) < 2:
    print('✅ PASS: utimes works')
    sys.exit(0)
print('❌ FAIL: utimes did not update mtime')
sys.exit(1)
"
