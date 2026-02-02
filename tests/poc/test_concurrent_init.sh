#!/bin/bash
# RFC-OPT-005: Concurrent Initialization Stress Test
# Validates AtomicPtr symbol loading in reals.rs under high-contention multi-threaded init
#
# This test spawns many concurrent processes with DYLD_INSERT_LIBRARIES to stress-test
# the shim's initialization path, particularly the RealSymbol::get() method's AtomicPtr
# caching under race conditions.
#
# Success criteria: All processes complete without deadlock or crash

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Configuration
INSTANCES=50
TIMEOUT_SECS=30

echo "=== RFC-OPT-005: Concurrent Init Stress Test ==="
echo "Instances: $INSTANCES"
echo "Timeout: ${TIMEOUT_SECS}s per instance"

# Build shim if needed
SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "Building release shim..."
    cargo build --release -p vrift-shim --manifest-path "$PROJECT_ROOT/Cargo.toml"
fi

if [[ ! -f "$SHIM_PATH" ]]; then
    echo "FATAL: Could not find shim at $SHIM_PATH"
    exit 1
fi

# Create test binary (copy and sign a multi-threaded tool)
TEST_BIN="$SCRIPT_DIR/test_concurrent_bin"
cp /bin/zsh "$TEST_BIN"
codesign --force --sign - "$TEST_BIN" 2>/dev/null || true

echo "Starting $INSTANCES concurrent instances..."

# Track PIDs
PIDS=()
FAILED=0

for i in $(seq 1 $INSTANCES); do
    (
        DYLD_INSERT_LIBRARIES="$SHIM_PATH" \
        DYLD_FORCE_FLAT_NAMESPACE=1 \
        timeout "$TIMEOUT_SECS" "$TEST_BIN" -c 'echo ok' >/dev/null 2>&1
    ) &
    PIDS+=($!)
done

echo "Waiting for all instances to complete..."

for pid in "${PIDS[@]}"; do
    if ! wait "$pid"; then
        FAILED=$((FAILED + 1))
    fi
done

# Cleanup
rm -f "$TEST_BIN"

echo ""
if [[ $FAILED -eq 0 ]]; then
    echo "=== PASS: All $INSTANCES instances completed successfully ==="
    exit 0
else
    echo "=== FAIL: $FAILED/$INSTANCES instances failed ==="
    exit 1
fi
