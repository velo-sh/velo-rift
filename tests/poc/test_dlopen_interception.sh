#!/bin/bash
# Test: dlopen Interception Behavior
# Priority: P1

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that dlopen works correctly under the shim

echo "=== Test: dlopen() Behavior ==="

# Use Python to test dlopen
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import ctypes
import sys
import os

try:
    # Attempt to load a common system library via dlopen (ctypes uses dlopen)
    if sys.platform == "darwin":
        lib_path = "/usr/lib/libSystem.B.dylib"
    else:
        lib_path = "libc.so.6"
        
    print(f"Attempting to dlopen {lib_path}...")
    lib = ctypes.CDLL(lib_path)
    
    if lib:
        print(f"✅ PASS: dlopen successfully loaded {lib_path}")
        sys.exit(0)
    else:
        print("❌ FAIL: dlopen returned null")
        sys.exit(1)
        
except Exception as e:
    print(f"dlopen error: {e}")
    sys.exit(1)
EOF
