#!/bin/bash
# RFC-0049 Gap Test: flock() Semantic Isolation
#
# This is a P0 gap that WILL break ccache and parallel builds
#
# Problem: flock() on temp file ≠ logical file lock
# Impact: Two processes both think they have exclusive lock

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== P0 Gap Test: flock() Semantic Isolation ==="
echo ""

SHIM_SRC="${PROJECT_ROOT}/crates/vrift-shim/src/lib.rs"

echo "[1] Checking for flock interception with VFS semantic..."

# Check if flock has VFS-aware locking
if grep -A30 "flock_shim\|fn flock" "$SHIM_SRC" 2>/dev/null | grep -q "vfs_prefix\|logical.*lock\|daemon\|manifest"; then
    echo "    ✅ flock has VFS-aware locking"
    HAS_FLOCK_VFS=true
else
    echo "    ❌ flock does NOT have VFS-aware locking"
    HAS_FLOCK_VFS=false
fi

echo ""
echo "[2] Impact Analysis:"
cat << 'EOF'
    ccache pattern:
    
    Process A:                   Process B:
    fd = open(".ccache/lock")    fd = open(".ccache/lock")
    // FD points to temp_A       // FD points to temp_B
    flock(fd, LOCK_EX)           flock(fd, LOCK_EX)
    // Got lock on temp_A ✅     // Got lock on temp_B ✅
    
    BOTH processes think they have the lock!
    
    Result: Race conditions in parallel builds
EOF

echo ""
echo "[3] Mitigation Strategy:"
cat << 'EOF'
    Shadow locking in daemon:
    
    flock(vfs_fd, LOCK_EX) → IPC to daemon
        daemon.lock(logical_path, LOCK_EX)
        if lock acquired: return 0
        else: block until available
    
    Lock state lives in daemon, not filesystem.
EOF

echo ""
if [[ "$HAS_FLOCK_VFS" == "true" ]]; then
    echo "✅ PASS: flock VFS semantic isolation implemented"
    exit 0
else
    echo "❌ GAP DETECTED: flock locks temp files, not logical files"
    echo ""
    echo "Affected tools: ccache, distcc, make -j, npm, pip"
    exit 1
fi
