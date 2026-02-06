#!/bin/bash
# Test: statx Interception Requirement
# Goal: Verify statx is intercepted (Linux extended stat API)
# Priority: P2 - Required for systemd and modern Linux tools

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

set -e
echo "=== Test: statx Interception Requirement ==="
echo ""

SHIM_PATH="${VRIFT_SHIM_PATH:-$(dirname "$0")/../../target/debug/libvrift_inception_layer.dylib}"

echo "[1] Platform Check:"
if [[ "$(uname)" == "Darwin" ]]; then
    echo "    ⚠️ macOS does not have statx syscall"
    echo "    This test is Linux-specific"
    echo ""
    echo "✅ PASS: statx not applicable on macOS"
    exit 0
fi

echo "[2] Checking Shim for statx Implementation:"
if [[ -f "$SHIM_PATH" ]]; then
    if nm -g "$SHIM_PATH" 2>/dev/null | grep -q "statx"; then
        echo "    ✅ statx symbol found in shim"
        STATX_IMPL=true
    else
        echo "    ❌ statx NOT intercepted in shim"
        STATX_IMPL=false
    fi
else
    echo "    ⚠️ Shim not built"
    STATX_IMPL=false
fi

echo ""
echo "[3] Impact Analysis:"
echo "    statx is used by:"
echo "    • systemd (for all file operations)"
echo "    • glibc >= 2.28 stat() wrapper"
echo "    • Modern file managers and tools"
echo ""

if [[ "$STATX_IMPL" == "true" ]]; then
    echo "✅ PASS: statx interception implemented"
    exit 0
else
    echo "❌ FAIL: statx interception NOT implemented"
    echo "   Priority: P2"
    echo "   Impact: systemd and modern Linux tools may not work correctly"
    exit 1
fi
