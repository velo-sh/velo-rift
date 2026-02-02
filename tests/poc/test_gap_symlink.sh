#!/bin/bash
# Compiler Gap Test: symlink
#
# RISK: MEDIUM - Library versioning uses symlinks
#
# EXPECTED BEHAVIOR:
# - symlink on VFS path should create Manifest symlink entry
# - readlink returns target from Manifest
#
# CURRENT: Passthrough (VFS symlinks fail)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Compiler Gap: symlink ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking for symlink interception..."
if grep -q "symlink_shim" "$SHIM_SRC" 2>/dev/null; then
    echo "    ✅ symlink intercepted"
    exit 0
else
    echo "    ❌ symlink NOT intercepted"
    echo ""
    echo "    Impact:"
    echo "    - ldconfig: libfoo.so → libfoo.so.1.0"
    echo "    - cmake install: creates versioned symlinks"
    echo "    - npm: node_modules/.bin symlinks"
    exit 1
fi
