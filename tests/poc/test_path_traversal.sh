#!/bin/bash
# test_path_traversal.sh - Stable Path Traversal Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/subdir/deep"
echo "secret" > "$TEST_DIR/secret.txt"

echo "=== Test: Path Traversal Behavior (Standalone) ==="

# We test with the shim loaded
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
python3 << EOF
import os
import sys

base = "$TEST_DIR/subdir/deep"
path = os.path.join(base, "../../secret.txt")

try:
    with open(path, 'r') as f:
        if f.read().strip() == "secret":
            print("✅ PASS: Path traversal normalized")
            sys.exit(0)
except Exception as e:
    print(f"❌ FAIL: {e}")
    sys.exit(1)
EOF

rm -rf "$TEST_DIR"
