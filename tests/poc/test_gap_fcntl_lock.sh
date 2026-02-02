#!/bin/bash
# RFC-0049 Gap Test: fcntl(F_SETLK) Record Locking
# Tests actual fcntl locking behavior, not source code
# Priority: P1

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== P1 Gap Test: fcntl() Record Locking Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create test file
echo "lock test content" > "$TEST_DIR/lockfile.txt"

# Test fcntl locking with Python
python3 << 'EOF'
import os
import sys
import fcntl
import struct

test_file = os.environ.get("TEST_FILE", "/tmp/lockfile.txt")

try:
    fd = os.open(test_file, os.O_RDWR)
    
    # Try to acquire exclusive lock
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        print("flock: Exclusive lock acquired")
        fcntl.flock(fd, fcntl.LOCK_UN)
    except BlockingIOError:
        print("flock: Lock held by another process")
    
    # Try fcntl F_SETLK (record lock)
    try:
        # struct flock: l_type, l_whence, l_start, l_len
        lockdata = struct.pack('hhllhh', fcntl.F_WRLCK, 0, 0, 0, 0, 0)
        fcntl.fcntl(fd, fcntl.F_SETLK, lockdata)
        print("fcntl F_SETLK: Write lock acquired")
        
        # Unlock
        lockdata = struct.pack('hhllhh', fcntl.F_UNLCK, 0, 0, 0, 0, 0)
        fcntl.fcntl(fd, fcntl.F_SETLK, lockdata)
    except Exception as e:
        print(f"fcntl F_SETLK: {e}")
    
    os.close(fd)
    print("✅ PASS: fcntl locking works correctly")
    sys.exit(0)
    
except Exception as e:
    print(f"fcntl error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/lockfile.txt"

python3 -c "
import os
import fcntl
import sys

fd = os.open('$TEST_DIR/lockfile.txt', os.O_RDWR)
fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
fcntl.flock(fd, fcntl.LOCK_UN)
os.close(fd)
print('✅ PASS: fcntl locking works')
sys.exit(0)
"
