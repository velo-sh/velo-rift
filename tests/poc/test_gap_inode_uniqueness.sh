#!/bin/bash
# RFC-0049 Gap Test: st_ino (Inode) Uniqueness
#
# Problem: CAS dedup means different logical files → same inode
# Impact: find -inum, rsync, git may show wrong results

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P1 Gap Test: st_ino Uniqueness ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[1] Checking for virtual inode generation..."

# Check if stat returns synthetic inodes
if grep -A30 "stat_common\|fn stat\|st_ino" "$SHIM_SRC" 2>/dev/null | grep -q "synthetic\|virtual.*ino\|hash.*path\|VFS_INODE"; then
    echo "    ✅ stat returns synthetic/virtual inodes"
    HAS_VIRTUAL_INO=true
else
    echo "    ❌ stat returns real CAS inodes (may duplicate)"
    HAS_VIRTUAL_INO=false
fi

echo ""
echo "[2] Impact Analysis:"
cat << 'EOF'
    CAS deduplication:
    
    project/
      foo.txt  → CAS blob abc123 → inode 12345
      bar.txt  → CAS blob abc123 → inode 12345 (same!)
      
    find -inum 12345      → finds BOTH files (wrong!)
    rsync --hard-links    → treats them as hard links (wrong!)
    git diff              → may skip comparison (wrong!)
EOF

echo ""
echo "[3] Mitigation Strategy:"
cat << 'EOF'
    Virtual inode generation:
    
    stat(vfs_path) {
        st_ino = hash(logical_path) % 2^32
        st_dev = VRIFT_VIRTUAL_DEV (fixed constant)
        st_nlink = 1 (always, even if CAS has many links)
    }
    
    Each logical path gets unique virtual inode.
EOF

echo ""
if [[ "$HAS_VIRTUAL_INO" == "true" ]]; then
    echo "✅ PASS: Virtual inode generation implemented"
    exit 0
else
    echo "❌ GAP DETECTED: Real CAS inodes exposed"
    echo ""
    echo "Affected tools: find, rsync, git, du"
    exit 1
fi
