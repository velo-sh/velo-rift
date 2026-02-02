#!/bin/bash
# RFC-0049 Gap Test: sendfile() Bypass
# Priority: P0
# Problem: Kernel zero-copy bypasses shim read/write

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P0 Gap Test: sendfile() Bypass ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -q "sendfile_shim\|sendfile.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ sendfile intercepted"
    exit 0
else
    echo "❌ GAP: sendfile NOT intercepted"
    echo ""
    echo "Impact: cp, rsync, nginx, web servers use sendfile"
    echo "        Kernel copies directly between FDs, bypasses shim"
    echo ""
    echo "Mitigation: Intercept sendfile, decompose to read()+write()"
    exit 1
fi
