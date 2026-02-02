#!/bin/bash
# RFC-0049 Gap Test: fchdir() Bypass
# Priority: P1
# Problem: fchdir(fd_from_vfs) bypasses chdir tracking

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P1 Gap Test: fchdir() Bypass ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -q "fchdir_shim\|fchdir.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ fchdir intercepted"
    exit 0
else
    echo "❌ GAP: fchdir NOT intercepted"
    echo ""
    echo "Impact: Parallel build tools use fd = open(dir); fchdir(fd)"
    echo "        Virtual CWD not updated, path resolution fails"
    exit 1
fi
