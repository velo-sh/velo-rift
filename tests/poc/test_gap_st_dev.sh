#!/bin/bash
# RFC-0049 Gap Test: st_dev (Device ID) Virtualization
# Priority: P2
# Problem: VFS files may show different st_dev than expected

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P2 Gap Test: st_dev Virtualization ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/syscalls/stat.rs"

if grep -q "st_dev.*=.*0x52494654\|RIFT" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ st_dev virtualized"
    exit 0
else
    echo "⚠️ GAP: st_dev not virtualized"
    echo ""
    echo "Impact: Cross-device rename detection"
    echo "        rename() may fail with EXDEV unexpectedly"
    exit 1
fi
