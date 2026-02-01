#!/bin/bash
# Test: access Permission Check Interception
# Goal: Verify access() is handled for VFS files
# Priority: P2 - Compilers and build tools use access() to check file existence

set -e
echo "=== Test: access Permission Check ==="
echo ""

SHIM_PATH="${VRIFT_SHIM_PATH:-$(dirname "$0")/../../target/debug/libvelo_shim.dylib}"

echo "[1] Checking Shim for access Implementation:"
if [[ -f "$SHIM_PATH" ]]; then
    if nm -gU "$SHIM_PATH" 2>/dev/null | grep -qE "_access(_shim)?$"; then
        echo "    ✅ access symbol found in shim"
        ACCESS_IMPL=true
    else
        echo "    ❌ access NOT intercepted in shim"
        ACCESS_IMPL=false
    fi
else
    echo "    ⚠️ Shim not built"
    ACCESS_IMPL=false
fi

echo ""
echo "[2] Impact Analysis:"
echo "    access() is used for:"
echo "    • Checking file existence (F_OK)"
echo "    • Checking read permission (R_OK)"
echo "    • Checking write permission (W_OK)"
echo "    • Checking execute permission (X_OK)"
echo ""
echo "    Compilers use access() to check if headers exist before including"
echo ""

echo "[3] Workaround:"
echo "    If access() is not intercepted, stat() is usually tried next"
echo "    Most tools fall back to stat() if access() fails"
echo ""

if [[ "$ACCESS_IMPL" == "true" ]]; then
    echo "✅ PASS: access interception implemented"
    exit 0
else
    echo "⚠️ access NOT intercepted (passthrough behavior)"
    echo "   Priority: P2"
    echo "   Impact: Some fast-path file existence checks may fail"
    exit 1
fi
