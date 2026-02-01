#!/bin/bash
# Test: Python Script Execution via VFS
# Goal: Python must successfully execute a script from VFS path
# Expected: FAIL - stat recursion deadlock
# Fixed: SUCCESS - Script runs and produces output

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# macOS hardened runtime blocks DYLD_INSERT_LIBRARIES on system binaries (python)
if [[ "$(uname)" == "Darwin" ]]; then
    echo "⏭️  SKIP: test_rust_cargo_build.sh - macOS hardened runtime blocks DYLD_INSERT_LIBRARIES"
    echo "       System binaries (cargo, rustc) cannot load the VFS shim on macOS."
    echo "       Run this test on Linux for full VFS E2E verification."
    exit 0
fi

echo "=== Test: Python Script Execution via VFS ==="
echo "Goal: python /vrift/project/main.py"
echo ""

# Setup (chflags first to handle leftover immutable files)
export VR_THE_SOURCE="/tmp/python_vfs_cas"
export VRIFT_VFS_PREFIX="/vrift"

# LMDB manifest is now in project's .vrift directory
export VRIFT_MANIFEST_DIR="/tmp/python_test_project/.vrift/manifest.lmdb"

chflags -R nouchg "$VR_THE_SOURCE" /tmp/python_test_project 2>/dev/null || true
rm -rf "$VR_THE_SOURCE" /tmp/python_test_project 2>/dev/null || true
mkdir -p "$VR_THE_SOURCE" /tmp/python_test_project

# Create a simple Python script
cat > /tmp/python_test_project/main.py << 'EOF'
#!/usr/bin/env python3
print("Hello from VFS Python!")
import sys
print(f"Python version: {sys.version}")
print(f"Script path: {__file__}")
EOF

cat > /tmp/python_test_project/utils.py << 'EOF'
def greet(name):
    return f"Hello, {name}!"
EOF

cat > /tmp/python_test_project/test_import.py << 'EOF'
#!/usr/bin/env python3
from utils import greet
print(greet("VFS User"))
EOF

echo "[STEP 1] Ingest Python project into VFS..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest /tmp/python_test_project --prefix /vrift/project

if [ ! -d "$VRIFT_MANIFEST_DIR" ]; then
    echo "[FAIL] Ingest failed - no LMDB manifest directory created"
    exit 1
fi
echo "[OK] LMDB Manifest created"

echo ""
echo "[STEP 2] Start daemon with manifest..."
killall vriftd 2>/dev/null || true
sleep 1
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/python_vfs_daemon.log 2>&1 &
DAEMON_PID=$!
sleep 2

echo "[STEP 3] Execute Python script via VFS..."
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"
export VRIFT_DEBUG=1

# Run Python in background with timeout
(
    python3 /vrift/project/main.py 2>&1
) &
PYTHON_PID=$!

# Wait with timeout (5 seconds)
sleep 5
if kill -0 $PYTHON_PID 2>/dev/null; then
    echo "[FAIL] Python execution TIMED OUT (5s) - likely stat recursion deadlock"
    kill -9 $PYTHON_PID 2>/dev/null
    unset DYLD_INSERT_LIBRARIES
    kill $DAEMON_PID 2>/dev/null
    exit 1
fi

wait $PYTHON_PID
PYTHON_EXIT=$?

unset DYLD_INSERT_LIBRARIES
kill $DAEMON_PID 2>/dev/null || true

if [ $PYTHON_EXIT -ne 0 ]; then
    echo "[FAIL] Python script failed with exit code $PYTHON_EXIT"
    echo "[INFO] Last 20 lines of daemon log:"
    tail -20 /tmp/python_vfs_daemon.log
    exit 1
fi

echo "[PASS] Python script executed successfully via VFS!"
EXIT_CODE=0

# Cleanup (chflags to remove immutable flags first)
chflags -R nouchg "$VR_THE_SOURCE" /tmp/python_test_project 2>/dev/null || true
rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" /tmp/python_test_project /tmp/python_vfs_daemon.log 2>/dev/null || true
exit $EXIT_CODE
