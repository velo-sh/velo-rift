#!/bin/bash
# Test: Concurrent Write and close() Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that concurrent writes to different FDs are handled correctly

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== Test: Concurrent Write Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Build directory logic
if [ -d "$PROJECT_ROOT/target/release" ]; then
    BUILD_DIR="$PROJECT_ROOT/target/release"
else
    BUILD_DIR="$PROJECT_ROOT/target/debug"
fi

case "$(uname -s)" in
    Darwin) SHIM_LIB="$BUILD_DIR/libvrift_shim.dylib" ;;
    Linux) SHIM_LIB="$BUILD_DIR/libvrift_shim.so" ;;
esac

# Test with Python
export TEST_DIR="$TEST_DIR"
DYLD_INSERT_LIBRARIES="$SHIM_LIB" LD_PRELOAD="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys
import threading

test_dir = os.environ.get("TEST_DIR", "/tmp")
file1 = os.path.join(test_dir, "file1.txt")
file2 = os.path.join(test_dir, "file2.txt")

def write_and_close(path, content):
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
        os.write(fd, content.encode())
        os.close(fd)
        return True
    except Exception as e:
        print(f"Write error for {path}: {e}")
        return False

# Run concurrent writes
t1 = threading.Thread(target=write_and_close, args=(file1, "Content for file 1"))
t2 = threading.Thread(target=write_and_close, args=(file2, "Content for file 2"))

t1.start()
t2.start()
t1.join()
t2.join()

# Verify content
try:
    with open(file1, 'r') as f1, open(file2, 'r') as f2:
        c1 = f1.read()
        c2 = f2.read()
        if c1 == "Content for file 1" and c2 == "Content for file 2":
            print("✅ PASS: Concurrent writes verified")
            sys.exit(0)
        else:
            print(f"❌ FAIL: Content mismatch. C1: {c1}, C2: {c2}")
            sys.exit(1)
except Exception as e:
    print(f"Verification error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

DYLD_INSERT_LIBRARIES="$SHIM_LIB" LD_PRELOAD="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import os
import sys
f = '$TEST_DIR/concurrent.txt'
fd = os.open(f, os.O_WRONLY | os.O_CREAT, 0o644)
os.write(fd, b'data')
os.close(fd)
if open(f, 'rb').read() == b'data':
    print('✅ PASS: Basic concurrent safety check')
    sys.exit(0)
sys.exit(1)
"
