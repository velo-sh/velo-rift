#!/bin/bash
# RFC-0047 P0 Test: open() Permission Mode Check
# 
# EXPECTED BEHAVIOR (per RFC-0047):
# - open(O_WRONLY) on a file with mode 0o444 should return EACCES
# - VFS should respect Manifest mode bits, not just allow all writes
#
# CURRENT BEHAVIOR (Bug):
# - open(O_WRONLY) does NOT check entry.mode before allowing writes

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0047 P0: open() Mode Check ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/interpose.rs"

# Verify the issue exists
echo "[1] Checking open_impl for mode check..."

# Search for mode check in write handling
if grep -A20 "is_write" "$SHIM_SRC" 2>/dev/null | grep -q "entry.mode\|EACCES\|permission"; then
    echo "    ✅ PASS: open_impl checks mode before write"
    HAS_MODE_CHECK=true
else
    echo "    ❌ FAIL: open_impl does NOT check mode before write"
    echo ""
    echo "    RFC-0047 Requirement:"
    echo "    open(O_WRONLY) on file with mode 0o444 should return EACCES"
    echo ""
    echo "    Current Behavior (Bug):"
    echo "    - Writes allowed regardless of Manifest mode"
    HAS_MODE_CHECK=false
fi

echo ""
echo "[2] Code verification:"

# Find write handling code
WRITE_HANDLING=$(grep -n "is_write\|O_WRONLY\|break_link" "$SHIM_SRC" 2>/dev/null | head -5)
if [[ -n "$WRITE_HANDLING" ]]; then
    echo "    Write handling found at:"
    echo "$WRITE_HANDLING" | sed 's/^/    /'
fi

echo ""
echo "[3] Expected Fix (per RFC-0047):"
cat << 'EOF'
    // In open_impl, before allowing write:
    if is_write {
        if let Some(entry) = state.query_manifest(resolved_path) {
            if (entry.mode & 0o200) == 0 {  // No write permission
                set_errno(libc::EACCES);
                return Some(-1);
            }
        }
    }
EOF

echo ""
if [[ "$HAS_MODE_CHECK" == "true" ]]; then
    echo "✅ PASS: RFC-0047 P0 mode check implemented"
    exit 0
else
    echo "❌ FAIL: RFC-0047 P0 mode check NOT implemented"
    echo ""
    echo "This test captures the expected behavior."
    echo "Implementation is tracked in RFC-0047-Syscall-Audit.md"
    exit 1
fi
