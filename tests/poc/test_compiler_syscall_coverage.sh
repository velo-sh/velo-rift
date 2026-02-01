#!/bin/bash
# Test: Compiler Syscall 100% Coverage Verification
# Goal: Verify ALL syscalls required by compilers are correctly intercepted
# Priority: CRITICAL - Must pass for compiler integration to work

set -e
echo "=== Compiler Syscall 100% Coverage Test ==="
echo "Date: $(date)"
echo ""

SHIM_PATH="${VRIFT_SHIM_PATH:-$(dirname "$0")/../../target/debug/libvelo_shim.dylib}"
if [[ ! -f "$SHIM_PATH" ]]; then
    SHIM_PATH="$(dirname "$0")/../../target/release/libvelo_shim.dylib"
fi

if [[ ! -f "$SHIM_PATH" ]]; then
    echo "⚠️ Shim not built at: $SHIM_PATH"
    exit 1
fi

echo "[1] Checking Shim Exported Symbols..."
echo "    Shim: $SHIM_PATH"
EXPORTED=$(nm -gU "$SHIM_PATH" 2>/dev/null || nm -g "$SHIM_PATH" 2>/dev/null)

echo ""
echo "[2] Critical Syscalls for Compilers:"
echo ""
printf "%-12s | %-6s | %-30s\n" "Syscall" "Status" "Purpose"
echo "-------------|--------|--------------------------------"

PASS=0
FAIL=0

check_syscall() {
    local syscall=$1
    local purpose=$2
    # Check for _syscall or _syscall_shim patterns
    if echo "$EXPORTED" | grep -qE " _${syscall}(_shim)?$"; then
        printf "%-12s | ✅ OK  | %s\n" "$syscall" "$purpose"
        ((PASS++))
    else
        printf "%-12s | ❌ MISS | %s\n" "$syscall" "$purpose"
        ((FAIL++))
    fi
}

# File metadata (CRITICAL for incremental builds)
check_syscall "stat" "mtime detection"
check_syscall "lstat" "symlink detection"
check_syscall "fstat" "fd metadata"

# File access
check_syscall "open" "file open"
check_syscall "close" "file close"
check_syscall "read" "file read"
check_syscall "write" "CoW write"

# Directory operations
check_syscall "opendir" "directory traversal"
check_syscall "readdir" "directory listing"
check_syscall "closedir" "directory close"

# Symbol resolution
check_syscall "readlink" "symlink resolution"

# Memory mapping (libraries)
check_syscall "mmap" "shared lib/large file"
check_syscall "munmap" "memory release"

# Dynamic loading
check_syscall "dlopen" "dynamic library load"
check_syscall "dlsym" "symbol resolution"

# Access checks
check_syscall "access" "permission check"

# File control
check_syscall "fcntl" "file flags"

echo ""
echo "[3] Summary:"
echo "    Passed: $PASS"
echo "    Failed: $FAIL"
echo "    Total:  $((PASS + FAIL))"
echo ""

# Critical syscalls that MUST pass
echo "[4] Critical Syscall Verification:"
CRITICAL_FAIL=0
for syscall in stat lstat fstat open read readlink; do
    if ! echo "$EXPORTED" | grep -qE " _${syscall}(_shim)?$"; then
        echo "    ❌ CRITICAL MISSING: $syscall"
        ((CRITICAL_FAIL++))
    fi
done

if [[ $CRITICAL_FAIL -gt 0 ]]; then
    echo ""
    echo "❌ CRITICAL FAILURE: $CRITICAL_FAIL core syscalls missing!"
    echo "   Compiler integration WILL NOT WORK."
    exit 1
fi

echo "    ✅ All critical syscalls present"
echo ""

echo "[5] Compiler Requirements:"
echo "    GCC/Clang: stat/lstat/fstat (mtime), open/read, mmap (.pch)"
echo "    Linker:    stat/fstat, mmap (.o/.a), readlink"
echo "    Native:    dlopen (.so/.dylib/.node), mmap"
echo ""

if [[ $FAIL -gt 0 ]]; then
    echo "⚠️ WARNING: $FAIL syscalls not intercepted"
    echo "   Some compiler scenarios may not work correctly."
    exit 1
else
    echo "✅ ALL PASS: 100% syscall coverage for compilers!"
    exit 0
fi
