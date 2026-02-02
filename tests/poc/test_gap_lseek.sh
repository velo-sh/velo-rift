#!/bin/bash
# Compiler Gap Test: lseek
#
# RISK: HIGH - Archive tools (ar, tar) require random access
#
# EXPECTED BEHAVIOR:
# - lseek on VFS FD should work correctly
# - Archive reading (ar rcs libfoo.a *.o) must work
#
# CURRENT: Passthrough (may cause inconsistent state)

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Compiler Gap: lseek ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking for lseek interception..."
if grep -q "lseek_shim\|lseek.*interpose" "$SHIM_SRC" 2>/dev/null; then
    echo "    ✅ lseek intercepted"
    exit 0
else
    echo "    ❌ lseek NOT intercepted"
    echo ""
    echo "    Impact:"
    echo "    - 'ar rcs lib.a *.o' random access archive creation"
    echo "    - 'objdump -d binary' seeking through ELF"
    echo "    - 'ld' linker reading sections"
    echo ""
    echo "    Why this matters:"
    echo "    - VFS files extracted to temp for reading"
    echo "    - If FD tracking lost, lseek goes to wrong file"
    exit 1
fi
