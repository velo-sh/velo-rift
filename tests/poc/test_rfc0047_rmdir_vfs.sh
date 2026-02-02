#!/bin/bash
# RFC-0047 P0 Test: rmdir() Mutation Semantics
#
# EXPECTED BEHAVIOR (per RFC-0047):
# - rmdir() on VFS path should remove Manifest directory entry
# - Should check directory is empty in Manifest
#
# CURRENT BEHAVIOR (Bug):
# - Returns EROFS for VFS paths

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0047 P0: rmdir() Mutation Semantics ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking rmdir_shim implementation..."

# Check if rmdir returns EROFS (current bug)
if grep -A15 "rmdir_shim\|fn rmdir" "$SHIM_SRC" 2>/dev/null | grep -q "EROFS\|Read-only"; then
    echo "    ❌ FAIL: rmdir_shim returns EROFS for VFS paths"
    RETURNS_EROFS=true
else
    echo "    ✅ rmdir_shim does not return EROFS"
    RETURNS_EROFS=false
fi

echo ""
echo "[2] Checking for Manifest removal..."

# Check if rmdir updates manifest
if grep -A20 "rmdir_shim\|fn rmdir" "$SHIM_SRC" 2>/dev/null | grep -q "manifest.*remove\|ManifestRemove"; then
    echo "    ✅ PASS: rmdir_shim removes Manifest entry"
    HAS_MANIFEST_OP=true
else
    echo "    ❌ FAIL: rmdir_shim does NOT update Manifest"
    HAS_MANIFEST_OP=false
fi

echo ""
echo "[3] Expected Behavior (per RFC-0047):"
cat << 'EOF'
    fn rmdir_shim(path: *const c_char) -> c_int {
        // 1. Check if VFS path
        // 2. Check directory is empty in Manifest
        // 3. Remove from Manifest: manifest.remove(path)
        // 4. Return 0 (success)
    }
EOF

echo ""
if [[ "$RETURNS_EROFS" == "false" ]] && [[ "$HAS_MANIFEST_OP" == "true" ]]; then
    echo "✅ PASS: RFC-0047 rmdir semantics implemented"
    exit 0
else
    echo "❌ FAIL: RFC-0047 rmdir semantics NOT implemented"
    exit 1
fi
