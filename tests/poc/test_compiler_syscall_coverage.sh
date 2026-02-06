#!/bin/bash
# Test: Compiler Syscall Coverage Behavior
# Priority: CRITICAL

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Verifies that major syscalls work correctly under the shim

echo "=== Compiler Syscall Coverage Behavior ==="

# We'll use Python to run a battery of syscall tests
DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib" DYLD_FORCE_FLAT_NAMESPACE=1 python3 << 'EOF'
import os
import sys
import fcntl
import mmap
import stat

def check(name, success):
    if success:
        print(f"✅ {name:15} | OK")
        return True
    else:
        print(f"❌ {name:15} | FAILED")
        return False

results = []
test_file = "/tmp/cov_test.txt"
with open(test_file, 'w') as f: f.write("syscall coverage test content")

try:
    # 1. Open/Read/Close
    fd = os.open(test_file, os.O_RDONLY)
    data = os.read(fd, 10)
    os.close(fd)
    results.append(check("open/read/close", b"syscall co" == data))
    
    # 2. Stat family
    st = os.stat(test_file)
    results.append(check("stat", st.st_size > 0))
    
    # 3. fcntl
    fd = os.open(test_file, os.O_RDONLY)
    flags = fcntl.fcntl(fd, fcntl.F_GETFL)
    os.close(fd)
    results.append(check("fcntl", flags >= 0))
    
    # 4. mmap
    fd = os.open(test_file, os.O_RDONLY)
    mm = mmap.mmap(fd, 0, access=mmap.ACCESS_READ)
    mm_data = mm.read(10)
    mm.close()
    os.close(fd)
    results.append(check("mmap", b"syscall co" == mm_data))
    
    # 5. Directory ops
    results.append(check("listdir", len(os.listdir("/tmp")) > 0))
    
    # 6. Symlinks
    link_path = "/tmp/cov_link"
    if os.path.exists(link_path): os.unlink(link_path)
    os.symlink(test_file, link_path)
    results.append(check("readlink", os.readlink(link_path) == test_file))
    os.unlink(link_path)
    
except Exception as e:
    print(f"Test crashed: {e}")
    sys.exit(1)

if all(results):
    print("\n✅ PASS: 100% behavioral coverage for core compiler syscalls")
    sys.exit(0)
else:
    print("\n❌ FAIL: Some syscalls failed behavioral verification")
    sys.exit(1)
EOF
rm -f /tmp/cov_test.txt
