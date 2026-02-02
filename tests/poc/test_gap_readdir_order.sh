#!/bin/bash
# RFC-0049 Gap Test: readdir() Order Consistency
# Priority: P2
# Problem: VFS readdir order may differ from real FS

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P2 Gap Test: readdir() Order Consistency ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -A30 "readdir_shim\|opendir_shim" "$SHIM_SRC" 2>/dev/null | grep -q "sort\|order\|consistent"; then
    echo "✅ readdir has consistent ordering"
    exit 0
else
    echo "⚠️ GAP: readdir order may vary"
    echo ""
    echo "Impact: Test frameworks, scripts expecting stable order"
    echo "        Usually not critical but may cause flaky tests"
    exit 1
fi
