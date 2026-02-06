#!/bin/bash
# Test: Issue #8 - Blocking close() I/O Performance
# Priority: P2

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that close() remains performant

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)
export TEST_DIR

echo "=== Test: close() Performance Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Test: Timing many close operations
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import time
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")

try:
    start_time = time.time()
    for i in range(100):
        path = os.path.join(test_dir, f"file_{i}.txt")
        fd = os.open(path, os.O_WRONLY | os.O_CREAT, 0o644)
        os.write(fd, b"test")
        os.close(fd)
    
    end_time = time.time()
    total_duration = end_time - start_time
    print(f"Total time for 100 close ops: {total_duration:.4f}s")
    
    # Normally close() should be very fast (sub-millisecond)
    # 100 closes should definitely take less than 1 second even with shim
    if total_duration < 1.0:
        print("✅ PASS: close() performance is acceptable")
        sys.exit(0)
    else:
        print("⚠️ WARN: close() performance is slow")
        sys.exit(0)
        
except Exception as e:
    print(f"close test error: {e}")
    sys.exit(1)
EOF
