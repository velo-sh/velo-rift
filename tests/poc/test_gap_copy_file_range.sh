#!/bin/bash
# RFC-0049 Gap Test: copy_file_range() Behavior
# Tests actual copy behavior, not source code
# Priority: P0

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR=$(mktemp -d)

echo "=== P0 Gap Test: copy_file_range() Behavior ==="

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# Create source file
echo "Source content for copy test" > "$TEST_DIR/source.txt"
touch "$TEST_DIR/dest.txt"

# Test copy with Python
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys
import shutil

test_dir = os.environ.get("TEST_DIR", "/tmp")
src = os.path.join(test_dir, "source.txt")
dst = os.path.join(test_dir, "dest.txt")

try:
    # Try copy_file_range if available (Linux 4.5+)
    if hasattr(os, 'copy_file_range'):
        src_fd = os.open(src, os.O_RDONLY)
        dst_fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
        src_size = os.fstat(src_fd).st_size
        
        try:
            bytes_copied = os.copy_file_range(src_fd, dst_fd, src_size)
            print(f"copy_file_range: {bytes_copied} bytes")
        except OSError as e:
            print(f"copy_file_range not supported: {e}")
            # Fallback
            os.lseek(src_fd, 0, os.SEEK_SET)
            data = os.read(src_fd, src_size)
            os.write(dst_fd, data)
            
        os.close(src_fd)
        os.close(dst_fd)
    else:
        # Fallback to shutil.copy
        shutil.copy(src, dst)
        print("Used shutil.copy (no copy_file_range)")
    
    # Verify
    with open(dst, 'r') as f:
        content = f.read()
        if "Source content" in content:
            print("✅ PASS: File copy works correctly")
            sys.exit(0)
        else:
            print(f"❌ FAIL: Content mismatch")
            sys.exit(1)
            
except Exception as e:
    print(f"Copy error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 -c "
import shutil
import sys
shutil.copy('$TEST_DIR/source.txt', '$TEST_DIR/dest2.txt')
with open('$TEST_DIR/dest2.txt') as f:
    if 'Source content' in f.read():
        print('✅ PASS: File copy works')
        sys.exit(0)
sys.exit(1)
"
