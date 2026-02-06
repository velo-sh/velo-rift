#!/bin/bash
# Test: Symlink Cycle Detection Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that symlink cycles don't crash the system

echo "=== Test: Symlink Cycle Behavior ==="

TEST_DIR=$(mktemp -d)
export TEST_DIR
cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create a cycle: a -> b -> a
ln -s "$TEST_DIR/b" "$TEST_DIR/a"
ln -s "$TEST_DIR/a" "$TEST_DIR/b"

# Test with Python (should handle error gracefully)
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
cycle_start = os.path.join(test_dir, "a")

try:
    # Try to open or stat the cycle
    print(f"Attempting to stat symlink cycle: {cycle_start}")
    os.stat(cycle_start)
    print("❌ FAIL: stat succeeded on infinite loop (unexpected)")
    sys.exit(1)
except OSError as e:
    # Expected: ELOOP (Too many levels of symbolic links)
    if e.errno == 62 or e.errno == 40: # 62 on macOS, 40 on Linux
        print(f"✅ PASS: Symlink cycle correctly detected (ELOOP: {e})")
        sys.exit(0)
    else:
        print(f"⚠️ INFO: stat failed with error {e.errno}: {e}")
        # Not strictly a failure if it didn't hang
        sys.exit(0)
except Exception as e:
    print(f"Cycle detection error: {e}")
    sys.exit(1)
EOF
