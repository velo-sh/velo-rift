#!/bin/bash
# Compiler Gap Test: ftruncate
#
# RISK: HIGH - GCC uses ftruncate when rewriting .o files
#
# EXPECTED BEHAVIOR:
# - ftruncate on VFS FD should update Manifest size
# - Content should be truncated in CoW temp file
#
# CURRENT: Passthrough (may corrupt VFS state)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Compiler Gap: ftruncate ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking for ftruncate interception..."
if grep -q "ftruncate_shim\|ftruncate.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "    ✅ ftruncate intercepted"
    exit 0
else
    echo "    ❌ ftruncate NOT intercepted"
    echo ""
    echo "    Impact:"
    echo "    - GCC: 'as' assembler truncates .o before write"
    echo "    - If VFS FD, truncate goes nowhere, file corrupted"
    echo ""
    echo "    Fix: Add ftruncate_shim that:"
    echo "    1. Check if FD is tracked VFS file"
    echo "    2. Truncate the CoW temp file"
    echo "    3. Update Manifest size on close"
    exit 1
fi
