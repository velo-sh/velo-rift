#!/bin/bash
# test_openat_faccessat.sh - Test directory-relative syscall handling
# Priority: P2 (Syscall Gap Detection)
set -e

echo "=== Test: openat/faccessat/fstatat Syscall Coverage ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SHIM_LIB="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"

echo "[1] Checking shim for *at() syscall symbols..."

check_symbol() {
    local sym="$1"
    if nm -gU "$SHIM_LIB" 2>/dev/null | grep -q "_${sym}_shim\|_${sym}$"; then
        echo "    ✓ $sym interception found"
        return 0
    else
        echo "    ✗ $sym NOT intercepted"
        return 1
    fi
}

FOUND=0
MISSING=0

if check_symbol "openat"; then ((FOUND++)); else ((MISSING++)); fi
if check_symbol "faccessat"; then ((FOUND++)); else ((MISSING++)); fi
if check_symbol "fstatat"; then ((FOUND++)); else ((MISSING++)); fi

echo ""
echo "[2] Why *at() syscalls matter:"
echo "    • Modern compilers use openat() with AT_FDCWD"
echo "    • faccessat() replaces access() in glibc 2.4+"
echo "    • fstatat() replaces stat() in multi-threaded apps"
echo "    • Ninja uses these for faster file operations"

echo ""
echo "[3] Fallback behavior:"
echo "    • If not intercepted, real syscall runs"
echo "    • VFS files will appear missing to caller"

echo ""
if [ "$MISSING" -eq 0 ]; then
    echo "✅ PASS: All *at() syscalls intercepted"
    exit 0
else
    echo "⚠️  GAP: $MISSING *at() syscalls not intercepted"
    echo "    Impact: Some compilers may not see VFS files"
    exit 1  # Fail to flag as gap
fi
