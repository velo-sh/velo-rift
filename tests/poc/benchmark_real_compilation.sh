#!/bin/bash
# Real-World Compilation Benchmark - Test on Velo Rift itself
# Measures shim interception overhead in actual multi-file Rust compilation

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Velo Rift Self-Compilation Benchmark ==="
echo "Testing shim overhead on real-world multi-file Rust project"
echo ""

cd "$PROJECT_ROOT"

SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "Building shim..."
    cargo build --release -p vrift-shim
fi

# Test on vrift-ipc (small, fast to compile)
TEST_PKG="vrift-ipc"

echo "📦 Test package: $TEST_PKG"
echo "🔥 Running 3 iterations..."
echo ""

BASELINE_TIMES=()
SHIM_TIMES=()

for iter in {1..3}; do
    echo "--- Iteration $iter ---"
    
    # Baseline
    cargo clean -p ${TEST_PKG} -q
    SECONDS=0
    cargo build -p ${TEST_PKG} --release -q
    BASELINE_S=$SECONDS
    BASELINE_TIMES+=($BASELINE_S)
    echo "Baseline: ${BASELINE_S}s"
    
    # With shim  
    cargo clean -p ${TEST_PKG} -q
    SECONDS=0
    VRIFT_DEBUG=0 DYLD_INSERT_LIBRARIES="$SHIM_PATH" \
        cargo build -p ${TEST_PKG} --release -q
    SHIM_S=$SECONDS
    SHIM_TIMES+=($SHIM_S)
    echo "With shim: ${SHIM_S}s"
    
    if command -v bc &>/dev/null; then
        OVERHEAD=$(echo "scale=2; (($SHIM_S - $BASELINE_S) / $BASELINE_S) * 100" | bc 2>/dev/null || echo "N/A")
        echo "Overhead: ${OVERHEAD}%"
    fi
    echo ""
done

# Calculate averages
BASELINE_AVG=$(( (${BASELINE_TIMES[0]} + ${BASELINE_TIMES[1]} + ${BASELINE_TIMES[2]}) / 3 ))
SHIM_AVG=$(( (${SHIM_TIMES[0]} + ${SHIM_TIMES[1]} + ${SHIM_TIMES[2]}) / 3 ))

echo "=== Final Results (3 iterations avg) ==="
echo "Baseline: ${BASELINE_AVG}s"
echo "With shim: ${SHIM_AVG}s"

if command -v bc &>/dev/null; then
    OVERHEAD=$(echo "scale=2; (($SHIM_AVG - $BASELINE_AVG) / $BASELINE_AVG) * 100" | bc 2>/dev/null || echo "N/A")
    echo "Average Overhead: ${OVERHEAD}%"
fi

echo ""
echo "📊 This tested:"
echo "- Real Rust project (vrift-ipc: ~10 files, 2K LOC)"
echo "- Multiple dependencies (serde, bincode, etc)"
echo "- Realistic multi-file compilation pattern"
echo "- Diverse FD access (not single-file loop)"
