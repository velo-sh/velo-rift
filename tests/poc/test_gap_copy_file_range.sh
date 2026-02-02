#!/bin/bash
# RFC-0049 Gap Test: copy_file_range() Bypass
# Priority: P0
# Problem: Kernel reflink/copy bypasses shim

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P0 Gap Test: copy_file_range() Bypass ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -q "copy_file_range_shim\|copy_file_range.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ copy_file_range intercepted"
    exit 0
else
    echo "❌ GAP: copy_file_range NOT intercepted"
    echo ""
    echo "Impact: cp --reflink, btrfs/zfs copy"
    echo "        Kernel copies directly, bypasses shim"
    exit 1
fi
