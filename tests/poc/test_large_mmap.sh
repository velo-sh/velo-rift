#!/bin/bash
# Test: large_mmap Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that mmap works for large memory regions

echo "=== Test: Large mmap Behavior ==="

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import mmap
import os
import sys

try:
    # Allocate 1MB anonymously
    mm = mmap.mmap(-1, 1024 * 1024)
    
    # Write to start, middle, end
    mm[0] = ord('A')
    mm[512 * 1024] = ord('B')
    mm[1024 * 1024 - 1] = ord('C')
    
    # Read back
    if mm[0] == ord('A') and mm[512 * 1024] == ord('B') and mm[1024 * 1024 - 1] == ord('C'):
        print("✅ PASS: Large anonymous mmap verified")
        mm.close()
        sys.exit(0)
    else:
        print("❌ FAIL: Memory content corruption")
        mm.close()
        sys.exit(1)
except Exception as e:
    print(f"Large mmap error: {e}")
    sys.exit(1)
EOF
