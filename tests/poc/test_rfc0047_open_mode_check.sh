#!/bin/bash
# RFC-0047 P0 Test: open() Permission Mode Check
# Tests actual permission checking behavior, not source code

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== RFC-0047 P0: open() Mode Check Behavior ==="
echo ""

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test open mode check with Python
python3 << 'EOF'
import os
import sys
import stat

test_dir = os.environ.get("TEST_DIR", "/tmp")
readonly_file = os.path.join(test_dir, "readonly.txt")

try:
    # Create file with content
    with open(readonly_file, 'w') as f:
        f.write("Original content")
    
    # Make it read-only (mode 0444)
    os.chmod(readonly_file, stat.S_IRUSR | stat.S_IRGRP | stat.S_IROTH)  # 0o444
    
    # Try to open for writing (should fail with EACCES)
    try:
        fd = os.open(readonly_file, os.O_WRONLY)
        os.close(fd)
        # If we get here, permission check failed
        print("❌ FAIL: Opened read-only file for writing")
        print("   Mode: 0o444 but O_WRONLY succeeded")
        # Restore permissions for cleanup
        os.chmod(readonly_file, 0o644)
        sys.exit(1)
    except PermissionError:
        print("✅ PASS: Permission check works correctly")
        print("   Cannot open 0o444 file with O_WRONLY (EACCES)")
        # Restore permissions for cleanup
        os.chmod(readonly_file, 0o644)
        sys.exit(0)
    except OSError as e:
        if e.errno == 13:  # EACCES
            print("✅ PASS: Permission check works (EACCES)")
            os.chmod(readonly_file, 0o644)
            sys.exit(0)
        else:
            print(f"Unexpected error: {e}")
            os.chmod(readonly_file, 0o644)
            sys.exit(1)
            
except Exception as e:
    print(f"Mode check error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import stat
import sys
f = '$TEST_DIR/ro.txt'
open(f, 'w').write('test')
os.chmod(f, 0o444)
try:
    fd = os.open(f, os.O_WRONLY)
    os.close(fd)
    os.chmod(f, 0o644)
    print('❌ FAIL: Opened readonly file')
    sys.exit(1)
except PermissionError:
    os.chmod(f, 0o644)
    print('✅ PASS: Mode check works')
    sys.exit(0)
"
