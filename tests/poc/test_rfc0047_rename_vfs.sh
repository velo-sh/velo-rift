#!/bin/bash
# RFC-0047 P0 Test: rename() Mutation Semantics
#
# EXPECTED BEHAVIOR (per RFC-0047):
# - rename() on VFS path should update Manifest path
# - CAS blob should remain unchanged (content-addressed)
#
# CURRENT BEHAVIOR (Bug):
# - Returns EROFS for VFS paths, breaking compilers

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0047 P0: rename() Mutation Semantics ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[1] Checking rename_shim implementation..."

# Check if rename returns EROFS (current bug)
if grep -A15 "rename_shim\|fn rename" "$SHIM_SRC" 2>/dev/null | grep -q "EROFS\|Read-only"; then
    echo "    ❌ FAIL: rename_shim returns EROFS for VFS paths"
    RETURNS_EROFS=true
else
    echo "    ✅ rename_shim does not return EROFS"
    RETURNS_EROFS=false
fi

echo ""
echo "[2] Checking for Manifest path update..."

# Check if rename updates manifest path
if grep -A20 "rename_shim\|fn rename" "$SHIM_SRC" 2>/dev/null | grep -q "manifest.*update\|ManifestRename\|update_path"; then
    echo "    ✅ PASS: rename_shim updates Manifest path"
    HAS_MANIFEST_OP=true
else
    echo "    ❌ FAIL: rename_shim does NOT update Manifest path"
    HAS_MANIFEST_OP=false
fi

echo ""
echo "[3] Expected Behavior (per RFC-0047):"
cat << 'EOF'
    fn rename_shim(old: *const c_char, new: *const c_char) -> c_int {
        // 1. Check if VFS paths
        // 2. Get entry from Manifest(old_path)
        // 3. Remove old entry, insert new: manifest.rename(old, new)
        // 4. Return 0 (success) - CAS blob unchanged
    }
EOF

echo ""
if [[ "$RETURNS_EROFS" == "false" ]] && [[ "$HAS_MANIFEST_OP" == "true" ]]; then
    echo "✅ PASS: RFC-0047 rename semantics implemented"
    exit 0
else
    echo "❌ FAIL: RFC-0047 rename semantics NOT implemented"
    echo ""
    echo "Impact: Compilers can't atomically replace files"
    exit 1
fi
