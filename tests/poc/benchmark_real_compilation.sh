#!/bin/bash
# Real-World Compilation Benchmark - Test shim overhead on multi-file C compilation
# Uses clang instead of cargo to avoid subprocess complexity issues with shim injection

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Multi-File Compilation Benchmark ==="
echo "Testing shim overhead on real-world multi-file compilation"
echo ""

SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_shim.dylib"
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "Building shim..."
    cargo build --release -p vrift-shim
    codesign -s - -f "$SHIM_PATH" 2>/dev/null || true
fi

TEST_DIR=$(mktemp -d)
trap "rm -rf $TEST_DIR" EXIT

# Create a realistic multi-file C project (simulates header-heavy codebase)
echo "📦 Creating test project with 50 source files..."
cd "$TEST_DIR"

# Create shared header
cat > common.h << 'EOF'
#ifndef COMMON_H
#define COMMON_H
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static inline int compute(int x) { return x * 2 + 1; }
#endif
EOF

# Create 50 source files
for i in $(seq 1 50); do
    cat > "module_$i.c" << EOF
#include "common.h"
int func_$i(int x) {
    return compute(x) + $i;
}
EOF
done

# Create main.c
cat > main.c << 'EOF'
#include "common.h"
int main(void) {
    printf("OK\n");
    return 0;
}
EOF

echo "✅ Created 50 source files + headers"
echo ""

# Compile all files
compile_all() {
    for i in $(seq 1 50); do
        cc -c -O2 "module_$i.c" -o "module_$i.o"
    done
    cc -c -O2 main.c -o main.o
    cc *.o -o program
}

echo "=== Test 1: Baseline (no shim) ==="
rm -f *.o program 2>/dev/null || true
SECONDS=0
compile_all
BASELINE_MS=$((SECONDS * 1000))
echo "Time: ${BASELINE_MS}ms"
./program
echo ""

echo "=== Test 2: With Shim ==="
rm -f *.o program 2>/dev/null || true
SECONDS=0
DYLD_INSERT_LIBRARIES="$SHIM_PATH" compile_all
SHIM_MS=$((SECONDS * 1000))
echo "Time: ${SHIM_MS}ms"
./program
echo ""

echo "=== Results ==="
echo "Baseline: ${BASELINE_MS}ms"
echo "With shim: ${SHIM_MS}ms"

if command -v bc &>/dev/null && [ "$BASELINE_MS" -gt 0 ]; then
    OVERHEAD=$(echo "scale=1; (($SHIM_MS - $BASELINE_MS) * 100 / $BASELINE_MS)" | bc 2>/dev/null || echo "N/A")
    echo "Overhead: ${OVERHEAD}%"
fi

echo ""
echo "📊 This tested:"
echo "- 50 C source files + headers"
echo "- 51 separate cc invocations"
echo "- Realistic multi-file compilation pattern"
echo "✅ Done"

