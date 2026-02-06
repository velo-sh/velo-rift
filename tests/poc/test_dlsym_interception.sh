#!/bin/bash
# Test: dlsym Interception Behavior
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that dlsym works correctly, especially for libc symbols

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=== Test: dlsym Behavior ==="

# Use Python to test dlsym behavior
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import ctypes
import sys
import os

try:
    # On macOS, use RTLD_DEFAULT or look up in libSystem.dylib
    if sys.platform == "darwin":
        libc = ctypes.CDLL(None)
    else:
        libc = ctypes.CDLL("libc.so.6")
        
    # Look up some basic symbols
    open_ptr = libc.open
    stat_ptr = libc.stat
    
    print(f"dlsym found open at: {open_ptr}")
    print(f"dlsym found stat at: {stat_ptr}")
    
    if open_ptr and stat_ptr:
        print("✅ PASS: dlsym successfully resolved core symbols")
        sys.exit(0)
    else:
        print("❌ FAIL: dlsym failed to resolve symbols")
        sys.exit(1)
        
except Exception as e:
    print(f"dlsym test error: {e}")
    sys.exit(1)
EOF
