#!/bin/bash
# RFC-0047 P2 Test: mkdir() VFS Semantics
#
# EXPECTED BEHAVIOR (per RFC-0047):
# - mkdir() should add directory entry to Manifest
# - No real directory created (pure virtual)
#
# CURRENT BEHAVIOR:
# - Passthrough to real filesystem

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0047 P2: mkdir() VFS Semantics ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking mkdir_shim implementation..."

# Check if mkdir handles VFS paths
if grep -A20 "mkdir_shim\|fn mkdir" "$SHIM_SRC" 2>/dev/null | grep -q "manifest.*insert\|ManifestUpsert\|add_dir"; then
    echo "    ✅ PASS: mkdir_shim creates Manifest entry"
    HAS_MANIFEST_OP=true
else
    echo "    ❌ FAIL: mkdir_shim does NOT create Manifest entry"
    HAS_MANIFEST_OP=false
fi

echo ""
echo "[2] Expected Behavior (per RFC-0047):"
cat << 'EOF'
    fn mkdir_shim(path: *const c_char, mode: mode_t) -> c_int {
        if is_vfs_path(path) {
            // Add directory entry to Manifest
            let entry = VnodeEntry { 
                mode: S_IFDIR | mode,
                size: 0,
                mtime: now(),
                hash: ZERO_HASH 
            };
            manifest_insert(path, entry);
            return 0;
        }
        real_mkdir(path, mode)
    }
EOF

echo ""
if [[ "$HAS_MANIFEST_OP" == "true" ]]; then
    echo "✅ PASS: RFC-0047 mkdir semantics implemented"
    exit 0
else
    echo "❌ FAIL: RFC-0047 mkdir semantics NOT implemented"
    echo ""
    echo "Current: Passthrough to real FS"
    exit 1
fi
