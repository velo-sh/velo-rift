#!/bin/bash
# Test: Inception Level 1 - Compile C Project via VFS
# Goal: GCC/Clang must successfully compile a C file accessed through the VFS shim
# Expected: FAIL (current state) - recursion/stat issues
# Fixed: SUCCESS - Binary produced and executes correctly

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Inception Test: Compile C Project via VFS ==="
echo "Goal: Fool Clang into believing virtual files are real."
echo ""

# Setup
export VR_THE_SOURCE="/tmp/inception_cas"
export VRIFT_MANIFEST="/tmp/inception.manifest"
export VRIFT_VFS_PREFIX="/vrift"

rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" /tmp/inception_project
mkdir -p "$VR_THE_SOURCE" /tmp/inception_project

# Create a simple C project
cat > /tmp/inception_project/hello.c << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from Inception VFS!\n");
    return 0;
}
EOF

echo "[STEP 1] Ingest project into VFS..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest /tmp/inception_project --output "$VRIFT_MANIFEST" --prefix /project

if [ ! -f "$VRIFT_MANIFEST" ]; then
    echo "[FAIL] Ingest failed - no manifest created"
    exit 1
fi
echo "[OK] Manifest created"

echo "[STEP 2] Start daemon..."
killall vriftd 2>/dev/null || true
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/inception_daemon.log 2>&1 &
DAEMON_PID=$!
sleep 2

echo "[STEP 3] Compile via VFS (with shim)..."
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"
export VRIFT_DEBUG=1

# Run compilation in background with timeout
(
    gcc /vrift/project/hello.c -o /tmp/inception_hello 2>&1
) &
COMPILE_PID=$!

# Wait with timeout
sleep 5
if kill -0 $COMPILE_PID 2>/dev/null; then
    echo "[FAIL] Compilation TIMED OUT (5s) - likely recursion deadlock"
    kill -9 $COMPILE_PID 2>/dev/null
    kill $DAEMON_PID 2>/dev/null
    exit 1
fi

wait $COMPILE_PID
COMPILE_EXIT=$?

unset DYLD_INSERT_LIBRARIES
kill $DAEMON_PID 2>/dev/null || true

if [ $COMPILE_EXIT -ne 0 ]; then
    echo "[FAIL] Compilation failed with exit code $COMPILE_EXIT"
    cat /tmp/inception_daemon.log | tail -20
    exit 1
fi

if [ ! -f /tmp/inception_hello ]; then
    echo "[FAIL] Binary not produced"
    exit 1
fi

echo "[STEP 4] Execute compiled binary..."
OUTPUT=$(/tmp/inception_hello)
if echo "$OUTPUT" | grep -q "Hello from Inception VFS"; then
    echo "[PASS] Binary executed correctly!"
    echo "Output: $OUTPUT"
    EXIT_CODE=0
else
    echo "[FAIL] Binary output unexpected: $OUTPUT"
    EXIT_CODE=1
fi

# Cleanup
rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" /tmp/inception_project /tmp/inception_hello /tmp/inception_daemon.log
exit $EXIT_CODE
