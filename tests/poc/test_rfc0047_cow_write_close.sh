#!/bin/bash
# RFC-0047 P1 Test: CoW Write Path (close() → CAS → Manifest)
#
# EXPECTED BEHAVIOR (per RFC-0047):
# 1. open(O_WRONLY) creates temp file, tracks FD
# 2. write() goes to temp file
# 3. close() hashes content → CAS insert → Manifest update
#
# CURRENT BEHAVIOR (Bug):
# - close() is passthrough, no CAS insert

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0047 P1: CoW Write Path ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

echo "[1] Checking close_shim implementation..."

# Check if close handles CoW reingest
if grep -A40 "close_shim" "$SHIM_SRC" 2>/dev/null | grep -q "reingest\|open_fds\|vpath\|temp_path"; then
    echo "    ✅ PASS: close_shim has CAS reingest logic"
    HAS_REINGEST=true
else
    echo "    ❌ FAIL: close_shim does NOT reingest to CAS"
    HAS_REINGEST=false
fi

echo ""
echo "[2] Checking FD tracking for dirty files..."

# Check if there's tracking for modified FDs
STATE_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/state.rs"
if grep -q "open_fds\|OpenFile" "$STATE_SRC" 2>/dev/null; then
    echo "    ✅ PASS: Dirty FD tracking exists"
    HAS_TRACKING=true
else
    echo "    ❌ FAIL: No dirty FD tracking found"
    HAS_TRACKING=false
fi

echo ""
echo "[3] Expected CoW Flow (per RFC-0047):"
cat << 'EOF'
    // 1. open(O_WRONLY)
    fn open_impl(path, flags) {
        if is_write {
            let temp_fd = create_temp_file();
            track_dirty_fd(temp_fd, original_path);
            return Some(temp_fd);
        }
    }
    
    // 2. close()
    fn close_impl(fd) {
        if is_dirty_fd(fd) {
            let hash = hash_file(temp_path);
            cas_insert(hash, temp_path);
            manifest_update(original_path, hash);
        }
        real_close(fd)
    }
EOF

echo ""
if [[ "$HAS_REINGEST" == "true" ]] && [[ "$HAS_TRACKING" == "true" ]]; then
    echo "✅ PASS: RFC-0047 CoW write path implemented"
    exit 0
else
    echo "❌ FAIL: RFC-0047 CoW write path NOT complete"
    exit 1
fi
