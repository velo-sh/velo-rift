#!/bin/bash
# RFC-0049 Gap Test: st_nlink (Hard Link Count) Virtualization
# Priority: P2
# Problem: CAS dedup exposes real nlink count

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P2 Gap Test: st_nlink Virtualization ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/syscalls/stat.rs"

if grep -q "st_nlink.*=.*1" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ st_nlink virtualized to 1"
    exit 0
else
    echo "⚠️ GAP: st_nlink shows real CAS link count"
    echo ""
    echo "Impact: rsync --hard-links, git, du"
    echo "        May treat unrelated files as hard-linked"
    exit 1
fi
