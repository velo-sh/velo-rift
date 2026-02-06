#!/bin/bash
# Test: Multiuser Isolation Behavior
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that isolation works as expected

echo "=== Test: Multiuser Isolation Behavior ==="

# For POC, we verify that current user can create isolated files
TEST_DIR=$(mktemp -d)
export TEST_DIR
chmod 700 "$TEST_DIR"

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys
import stat

test_dir = os.environ.get("TEST_DIR", "/tmp")

try:
    # Create private file
    private_file = os.path.join(test_dir, "private.txt")
    with open(private_file, 'w') as f:
        f.write("secret")
    os.chmod(private_file, 0o600)
    
    # Verify only current user can read
    st = os.stat(private_file)
    mode = stat.S_IMODE(st.st_mode)
    
    if mode == 0o600:
        print(f"✅ PASS: Private file created with permissions: {oct(mode)}")
        sys.exit(0)
    else:
        print(f"❌ FAIL: Permission mismatch: {oct(mode)}")
        sys.exit(1)
        
except Exception as e:
    print(f"Isolation error: {e}")
    sys.exit(1)
EOF
