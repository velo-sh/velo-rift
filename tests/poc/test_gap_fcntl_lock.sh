#!/bin/bash
# RFC-0049 Gap Test: fcntl(F_SETLK) Record Locking
# Priority: P1
# Problem: POSIX record locks applied to temp file, not logical file

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P1 Gap Test: fcntl() Record Locking ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -A30 "fcntl_shim\|fn fcntl" "$SHIM_SRC" 2>/dev/null | grep -q "F_SETLK\|F_GETLK\|vfs.*lock"; then
    echo "✅ fcntl has VFS-aware locking"
    exit 0
else
    echo "❌ GAP: fcntl locking NOT VFS-aware"
    echo ""
    echo "Impact: Database files, npm/pip package managers"
    echo "        Lock applied to temp, not logical file"
    exit 1
fi
