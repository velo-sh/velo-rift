#!/bin/bash
# test_zero_byte_file.sh - Robust Zero-Byte Behavior
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_DIR=$(mktemp -d)
touch "$TEST_DIR/zero.txt"

echo "=== Test: Zero-Byte File Behavior (Standalone) ==="

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
python3 << EOF
import os
import sys

path = "$TEST_DIR/zero.txt"
try:
    st = os.stat(path)
    if st.st_size == 0:
        print("✅ PASS: Zero-byte file stat works")
        sys.exit(0)
except Exception as e:
    print(f"❌ FAIL: {e}")
    sys.exit(1)
EOF

rm -rf "$TEST_DIR"
