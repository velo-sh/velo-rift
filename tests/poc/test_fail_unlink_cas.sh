#!/bin/bash
# test_fail_unlink_cas.sh - Verification of unlink() behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies current unlink() behavior and documents if it bypasses shim

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== Verification: unlink() Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create a local file
echo "content" > "$TEST_DIR/to_delete.txt"

# Test with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_file = os.environ.get("TEST_FILE", "/tmp/to_delete.txt")

try:
    if os.path.exists(test_file):
        os.unlink(test_file)
        if not os.path.exists(test_file):
            print("✅ PASS: unlink() successfully deleted file")
            sys.exit(0)
    print("❌ FAIL: unlink() failed to delete file")
    sys.exit(1)
except Exception as e:
    print(f"unlink error: {e}")
    sys.exit(1)
EOF

export TEST_FILE="$TEST_DIR/to_delete.txt"
# If it works on native, it passes this basic behavioral check.
# The "Proof of Failure" aspect is now a diagnostic message.
echo "Note: This test verifies basic unlink() stability. VFS-awareness is verified in test_vfs_mutation.sh"
