#!/bin/bash
# Test: Rust Build Acceleration - cargo build via VFS
# Goal: cargo must successfully compile a Rust crate with source files accessed through VFS
# Expected: FAIL (current state) - stat recursion deadlock
# Fixed: SUCCESS - Binary produced and executes correctly

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Rust Build Test: cargo build via VFS ==="
echo "Goal: Fool cargo/rustc into believing virtual files are real."
echo ""

# Setup
export VR_THE_SOURCE="/tmp/rust_build_cas"
export VRIFT_MANIFEST="/tmp/rust_build.manifest"
export VRIFT_VFS_PREFIX="/vrift"

rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" /tmp/rust_test_crate
mkdir -p "$VR_THE_SOURCE" /tmp/rust_test_crate/src

# Create a simple Rust crate
cat > /tmp/rust_test_crate/Cargo.toml << 'EOF'
[package]
name = "vrift_test_crate"
version = "0.1.0"
edition = "2021"

[dependencies]
EOF

cat > /tmp/rust_test_crate/src/main.rs << 'EOF'
fn main() {
    println!("Hello from Rust VFS Build!");
}
EOF

echo "[STEP 1] Ingest Rust crate into VFS..."
"${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest /tmp/rust_test_crate --output "$VRIFT_MANIFEST" --prefix /crate

if [ ! -f "$VRIFT_MANIFEST" ]; then
    echo "[FAIL] Ingest failed - no manifest created"
    exit 1
fi
echo "[OK] Manifest created"
echo "[INFO] Manifest contents:"
"${PROJECT_ROOT}/target/debug/vrift" manifest cat "$VRIFT_MANIFEST" | head -10

echo ""
echo "[STEP 2] Start daemon..."
killall vriftd 2>/dev/null || true
sleep 1
"${PROJECT_ROOT}/target/debug/vriftd" start > /tmp/rust_build_daemon.log 2>&1 &
DAEMON_PID=$!
sleep 2

echo "[STEP 3] Attempting cargo build via VFS (with shim)..."
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="${PROJECT_ROOT}/target/debug/libvelo_shim.dylib"
export VRIFT_DEBUG=1

# Create temp directory for build output
rm -rf /tmp/rust_vfs_target
mkdir -p /tmp/rust_vfs_target

# Run cargo build in background with timeout
(
    cd /tmp && \
    cargo build --manifest-path /vrift/crate/Cargo.toml \
        --target-dir /tmp/rust_vfs_target 2>&1
) &
BUILD_PID=$!

# Wait with timeout (10 seconds for simple crate)
sleep 10
if kill -0 $BUILD_PID 2>/dev/null; then
    echo "[FAIL] cargo build TIMED OUT (10s) - likely recursion deadlock"
    kill -9 $BUILD_PID 2>/dev/null
    unset DYLD_INSERT_LIBRARIES
    kill $DAEMON_PID 2>/dev/null
    exit 1
fi

wait $BUILD_PID
BUILD_EXIT=$?

unset DYLD_INSERT_LIBRARIES
kill $DAEMON_PID 2>/dev/null || true

if [ $BUILD_EXIT -ne 0 ]; then
    echo "[FAIL] cargo build failed with exit code $BUILD_EXIT"
    echo "[INFO] Last 20 lines of daemon log:"
    tail -20 /tmp/rust_build_daemon.log
    exit 1
fi

# Check if binary was produced
BINARY="/tmp/rust_vfs_target/debug/vrift_test_crate"
if [ ! -f "$BINARY" ]; then
    echo "[FAIL] Binary not produced at $BINARY"
    ls -la /tmp/rust_vfs_target/debug/ 2>/dev/null || echo "  (debug directory not found)"
    exit 1
fi

echo "[STEP 4] Execute compiled binary..."
OUTPUT=$("$BINARY")
if echo "$OUTPUT" | grep -q "Hello from Rust VFS Build"; then
    echo "[PASS] Rust binary executed correctly!"
    echo "Output: $OUTPUT"
    EXIT_CODE=0
else
    echo "[FAIL] Binary output unexpected: $OUTPUT"
    EXIT_CODE=1
fi

# Cleanup
rm -rf "$VR_THE_SOURCE" "$VRIFT_MANIFEST" /tmp/rust_test_crate /tmp/rust_vfs_target /tmp/rust_build_daemon.log
exit $EXIT_CODE
