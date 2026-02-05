#!/bin/bash
set -u

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Auto-detect target directory (prefer release)
if [ -d "$PROJECT_ROOT/target/release" ]; then
    TARGET_DIR="release"
else
    TARGET_DIR="debug"
fi

SHIM_BIN="$PROJECT_ROOT/target/$TARGET_DIR/libvrift_shim.dylib"
VRIFTD_BIN="$PROJECT_ROOT/target/$TARGET_DIR/vriftd"
TEST_DIR="$PROJECT_ROOT/test_resilience_work"

mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Compile helper
gcc -O2 "$PROJECT_ROOT/scripts/simple_open.c" -o simple_open

# Cleanup
pkill -f vriftd || true

echo "--- Resilience Test: Log Levels ---"

# 1. Test Log Level: Error (Info logs should be suppressed)
echo "Testing VRIFT_LOG_LEVEL=error..."
export VRIFT_LOG_LEVEL=error
export VRIFT_DEBUG=1
export DYLD_INSERT_LIBRARIES="$SHIM_BIN"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_VFS_PREFIX="/vfs"
export VRIFT_MANIFEST="$TEST_DIR/nonexistent.lmdb"

# This should trigger some INFO logs during init, but they should be suppressed if level check works
./simple_open /vfs/test.txt 2> logs_error.txt || true

if grep -q "\[INFO\]" logs_error.txt; then
    echo "❌ Fail: INFO logs found when level set to error"
else
    echo "✅ Success: INFO logs suppressed"
fi

# 2. Test Log Level: Trace (Should show everything)
echo "Testing VRIFT_LOG_LEVEL=debug..."
VRIFT_DEBUG=1 VRIFT_LOG_LEVEL=debug DYLD_INSERT_LIBRARIES=$SHIM_BIN ./test_resilience_work/simple_open 1 2> logs_debug.txt
if grep -q "VFS HIT" logs_debug.txt; then
    echo "✅ Success: DEBUG logs present"
else
    echo "❌ Fail: DEBUG logs missing"
    cat logs_debug.txt
fi

echo ""
echo "--- Resilience Test: Circuit Breaker ---"

# 3. Test Circuit Breaker: threshold=2
echo "Testing Circuit Breaker (threshold=2)..."
export VRIFT_LOG_LEVEL=info
export VRIFT_CIRCUIT_BREAKER_THRESHOLD=2
export VRIFT_DEBUG=1

# Run 5 iterations in ONE process
./simple_open /vfs/test_loop.txt 5 2> logs_cb.txt || true

if grep -q "CIRCUIT BREAKER TRIPPED" logs_cb.txt; then
    echo "✅ Success: Circuit breaker tripped message found in log"
else
    echo "❌ Fail: Circuit breaker trip message NOT found"
    cat logs_cb.txt
fi

# Count connect failures
# Circuit breaker trips after threshold (2) failures.
# Depending on whether the 3rd call is blocked BEFORE or AFTER the increment, it might be 2 or 3.
FAIL_COUNT=$(grep -c "DAEMON CONNECTION FAILED" logs_cb.txt)
if [ "$FAIL_COUNT" -ge 2 ]; then
    echo "✅ Success: Circuit breaker tripped after $FAIL_COUNT attempts"
else
    echo "❌ Fail: Found $FAIL_COUNT connect attempts, expected >= 2"
    cat logs_cb.txt
fi

echo "--- Resilience Test Complete ---"
# rm -rf "$TEST_DIR" # Keep for inspection
