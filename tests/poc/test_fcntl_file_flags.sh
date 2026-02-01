#!/bin/bash
# Test: fcntl File Flags Interception
# Goal: Verify fcntl is handled for VFS files
# Priority: P3 - Some tools use fcntl for file locking and flags

set -e
echo "=== Test: fcntl File Flags ==="
echo ""

SHIM_PATH="${VRIFT_SHIM_PATH:-$(dirname "$0")/../../target/debug/libvelo_shim.dylib}"

echo "[1] Checking Shim for fcntl Implementation:"
if [[ -f "$SHIM_PATH" ]]; then
    if nm -gU "$SHIM_PATH" 2>/dev/null | grep -qE "_fcntl(_shim)?$"; then
        echo "    ✅ fcntl symbol found in shim"
        FCNTL_IMPL=true
    else
        echo "    ❌ fcntl NOT intercepted in shim"
        FCNTL_IMPL=false
    fi
else
    echo "    ⚠️ Shim not built"
    FCNTL_IMPL=false
fi

echo ""
echo "[2] Impact Analysis:"
echo "    fcntl is used for:"
echo "    • F_GETFL/F_SETFL - get/set file flags"
echo "    • F_GETFD/F_SETFD - get/set fd flags (FD_CLOEXEC)"
echo "    • F_GETLK/F_SETLK - file locking"
echo ""
echo "    Most common use: checking if file is open for read/write"
echo ""

echo "[3] Current Behavior:"
echo "    Without interception, fcntl passes through to real syscall"
echo "    This generally works IF the underlying fd is valid"
echo ""

if [[ "$FCNTL_IMPL" == "true" ]]; then
    echo "✅ PASS: fcntl interception implemented"
    exit 0
else
    echo "⚠️ fcntl NOT intercepted (passthrough behavior)"
    echo "   Priority: P3"
    echo "   Impact: File locking may not work on pure VFS files"
    exit 1
fi
