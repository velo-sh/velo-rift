#!/bin/bash
# RFC-0047 P0 Test: unlink() Mutation Semantics
#
# EXPECTED BEHAVIOR (per RFC-0047):
# - unlink() on VFS path should remove Manifest entry
# - Should check write permission on parent directory
# - CAS blob should remain unchanged (immutable)
#
# CURRENT BEHAVIOR (Bug):
# - Returns EROFS for VFS paths, breaking compilers

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0047 P0: unlink() Mutation Semantics ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking unlink_shim implementation..."

# Check if unlink returns EROFS (current bug)
if grep -A15 "unlink_shim\|fn unlink" "$SHIM_SRC" 2>/dev/null | grep -q "EROFS\|Read-only"; then
    echo "    ❌ FAIL: unlink_shim returns EROFS for VFS paths"
    echo ""
    echo "    RFC-0047 Requirement:"
    echo "    unlink() should remove Manifest entry, not return EROFS"
    RETURNS_EROFS=true
else
    echo "    ✅ unlink_shim does not return EROFS"
    RETURNS_EROFS=false
fi

echo ""
echo "[2] Checking for Manifest removal..."

# Check if unlink calls manifest.remove or similar
if grep -A20 "unlink_shim\|fn unlink" "$SHIM_SRC" 2>/dev/null | grep -q "manifest.*remove\|ManifestRemove\|remove_entry"; then
    echo "    ✅ PASS: unlink_shim calls Manifest removal"
    HAS_MANIFEST_OP=true
else
    echo "    ❌ FAIL: unlink_shim does NOT update Manifest"
    HAS_MANIFEST_OP=false
fi

echo ""
echo "[3] Finding current implementation..."
UNLINK_LINE=$(grep -n "unlink_shim\|pub.*fn.*unlink" "$SHIM_SRC" 2>/dev/null | head -1)
if [[ -n "$UNLINK_LINE" ]]; then
    echo "    Found at: $UNLINK_LINE"
fi

echo ""
echo "[4] Expected Behavior (per RFC-0047):"
cat << 'EOF'
    fn unlink_shim(path: *const c_char) -> c_int {
        // 1. Check if VFS path
        // 2. Check write permission on parent (entry.mode)
        // 3. Remove from Manifest: manifest.remove(path)
        // 4. Return 0 (success) - CAS blob untouched
    }
EOF

echo ""
if [[ "$RETURNS_EROFS" == "false" ]] && [[ "$HAS_MANIFEST_OP" == "true" ]]; then
    echo "✅ PASS: RFC-0047 unlink semantics implemented"
    exit 0
else
    echo "❌ FAIL: RFC-0047 unlink semantics NOT implemented"
    echo ""
    echo "Impact: Compilers can't delete .o files during rebuild"
    exit 1
fi
