#!/bin/bash
# RFC-0049 Gap Test: dup/dup2 FD Tracking
# Priority: P1
# Problem: dup(vfs_fd) may create untracked FD

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P1 Gap Test: dup/dup2 FD Tracking ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -q "dup_shim\|dup2_shim\|dup.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ dup/dup2 intercepted with VFS tracking"
    exit 0
else
    echo "❌ GAP: dup/dup2 NOT tracked for VFS FDs"
    echo ""
    echo "Impact: Shell redirection (exec 3<file), subprocess FD inheritance"
    echo "        dup'ed FD loses VFS tracking"
    exit 1
fi
