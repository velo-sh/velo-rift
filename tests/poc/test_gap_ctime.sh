#!/bin/bash
# RFC-0049 Gap Test: ctime (Change Time) Update
# Priority: P2
# Problem: chmod/chown should update ctime, not mtime

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P2 Gap Test: ctime (Change Time) Update ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -A20 "chmod_shim\|fchmod_shim" "$SHIM_SRC" 2>/dev/null | grep -q "ctime\|change.*time"; then
    echo "✅ ctime updated on metadata change"
    exit 0
else
    echo "⚠️ GAP: ctime not updated separately from mtime"
    echo ""
    echo "Impact: Make, git 'was metadata changed?' checks"
    echo "        Usually not critical"
    exit 1
fi
