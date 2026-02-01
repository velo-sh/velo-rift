#!/bin/bash
# Compiler Gap Test: utimes/futimes
#
# RISK: HIGH - Make/Ninja use this for dependency tracking
#
# EXPECTED BEHAVIOR:
# - utimes on VFS path should update Manifest mtime
# - Incremental builds must see correct mtime
#
# CURRENT: Passthrough (VFS mtime never updated)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Compiler Gap: utimes/futimes ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[1] Checking for utimes interception..."
HAS_UTIMES=false
HAS_FUTIMES=false

if grep -q "utimes_shim\|utimes.*interpose\|utimensat_shim" "$SHIM_SRC" 2>/dev/null; then
    echo "    ✅ utimes intercepted"
    HAS_UTIMES=true
else
    echo "    ❌ utimes NOT intercepted"
fi

if grep -q "futimes_shim\|futimes.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "    ✅ futimes intercepted"
    HAS_FUTIMES=true
else
    echo "    ❌ futimes NOT intercepted"
fi

echo ""
echo "[2] Impact Analysis:"
echo "    - 'make' uses stat mtime for rebuild decisions"
echo "    - 'touch file.o' should update VFS mtime"
echo "    - If passthrough: VFS mtime unchanged → stale builds"
echo ""
echo "    Common commands affected:"
echo "    - make touch"
echo "    - ninja -t touch"
echo "    - touch -t timestamp file"

if [[ "$HAS_UTIMES" == "true" ]]; then
    echo ""
    echo "✅ PASS: utimes/futimes intercepted"
    exit 0
else
    echo ""
    echo "❌ FAIL: utimes/futimes NOT intercepted"
    exit 1
fi
