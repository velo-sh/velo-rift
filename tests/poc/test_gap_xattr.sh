#!/bin/bash
# RFC-0049 Gap Test: xattr (Extended Attributes)
# Priority: P3
# Problem: getxattr/setxattr on VFS files

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P3 Gap Test: xattr (Extended Attributes) ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

if grep -q "setxattr_shim\|removexattr_shim" "$SHIM_SRC" 2>/dev/null; then
    echo "✅ xattr intercepted"
    exit 0
else
    echo "⚠️ GAP: xattr NOT intercepted"
    echo ""
    echo "Impact: macOS code signing, Finder tags, ACLs"
    echo "        Usually not critical for compilers"
    exit 1
fi
