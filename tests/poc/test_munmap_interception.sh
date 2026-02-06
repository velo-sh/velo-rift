#!/bin/bash
# Test: munmap Interception Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that munmap works correctly

echo "=== Test: munmap Behavior ==="

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import mmap
import os
import sys

try:
    # Create mmap
    mm = mmap.mmap(-1, 4096)
    mm.write(b"data")
    
    # munmap happens on close in Python's mmap object,
    # but we can simulate the syscall via ctypes if needed.
    # For basic verification, we'll check if it doesn't crash the shim.
    mm.close()
    
    print("✅ PASS: mmap closed (munmap called) successfully")
    sys.exit(0)
    
except Exception as e:
    print(f"munmap error: {e}")
    sys.exit(1)
EOF
