#!/bin/bash
# RFC-0049 Gap Test: mmap(MAP_SHARED) Write Tracking
#
# This is a P0 gap that WILL break Git and databases
#
# Problem: mmap(MAP_SHARED) + write bypasses the shim
# Impact: Changes not tracked for CAS reingest

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P0 Gap Test: mmap(MAP_SHARED) Write Tracking ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[1] Checking for mmap interception with write detection..."

# Check if mmap has write tracking
if grep -A30 "mmap_impl\|fn mmap" "$SHIM_SRC" 2>/dev/null | grep -q "MAP_SHARED\|PROT_WRITE\|dirty\|track.*write"; then
    echo "    ✅ mmap has MAP_SHARED write tracking"
    HAS_MMAP_TRACKING=true
else
    echo "    ❌ mmap does NOT track MAP_SHARED writes"
    HAS_MMAP_TRACKING=false
fi

echo ""
echo "[2] Impact Analysis:"
cat << 'EOF'
    Git pack-objects pattern:
    
    fd = open(".git/objects/pack/pack-xxx.pack", O_RDWR);
    map = mmap(NULL, size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
    memcpy(map + offset, data, len);  // ❌ Write bypasses shim!
    msync(map, size, MS_SYNC);
    munmap(map, size);
    
    Result: VFS doesn't know file changed.
    Git status may show wrong results.
EOF

echo ""
echo "[3] Mitigation Strategy:"
cat << 'EOF'
    Option A: Intercept msync() and re-hash file
    Option B: Convert MAP_SHARED → MAP_PRIVATE + CoW
    Option C: Detect and warn (not transparent)
EOF

echo ""
if [[ "$HAS_MMAP_TRACKING" == "true" ]]; then
    echo "✅ PASS: mmap MAP_SHARED tracking implemented"
    exit 0
else
    echo "❌ GAP DETECTED: mmap MAP_SHARED writes not tracked"
    echo ""
    echo "Affected tools: Git, SQLite, LMDB, mmap-based databases"
    exit 1
fi
