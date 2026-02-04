#!/bin/bash
# RFC-0047 Critical E2E Test: GCC Compile Cycle
#
# This test runs a FULL GCC compilation in VFS mode.
# If vrift breaks compilers, this test WILL fail.
#
# GCC compile cycle:
# 1. cc1: preprocess + compile → .s
# 2. as: assemble → .o (may unlink existing)
# 3. ld: link → executable
# 4. May use rename for atomic output

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== E2E Test: GCC Compile in VFS ==="
echo ""

# Check for GCC
if ! command -v gcc &>/dev/null; then
    echo "⚠️ GCC not found, skipping"
    exit 0
fi

# Build
echo "[1] Building components..."
(cd "$PROJECT_ROOT" && cargo build -p vrift-shim -p vrift-cli -p vrift-daemon 2>/dev/null)

SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_shim.dylib"
CLI_PATH="${PROJECT_ROOT}/target/debug/vrift"
DAEMON_PATH="${PROJECT_ROOT}/target/debug/vriftd"

# Create test project
TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/project/src"

# Create source files
cat > "$TEST_DIR/project/src/main.c" << 'CEOF'
#include <stdio.h>
#include "helper.h"

int main() {
    printf("Hello from VFS compile test!\n");
    helper_function();
    return 0;
}
CEOF

cat > "$TEST_DIR/project/src/helper.h" << 'CEOF'
#ifndef HELPER_H
#define HELPER_H
void helper_function(void);
#endif
CEOF

cat > "$TEST_DIR/project/src/helper.c" << 'CEOF'
#include <stdio.h>
#include "helper.h"

void helper_function(void) {
    printf("Helper called!\n");
}
CEOF

cat > "$TEST_DIR/project/Makefile" << 'CEOF'
CC = gcc
CFLAGS = -Wall -g
OBJS = src/main.o src/helper.o
TARGET = myprogram

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) $(CFLAGS) -o $@ $(OBJS)

src/%.o: src/%.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJS) $(TARGET)

.PHONY: all clean
CEOF

echo "[2] Created test project: $TEST_DIR/project"
ls -la "$TEST_DIR/project/src/"

# Ingest
echo ""
echo "[3] Ingesting project..."
cd "$TEST_DIR/project"
"$CLI_PATH" ingest . 2>&1 | grep -E "Complete|files|Manifest" | head -3 || true
MANIFEST_PATH="$TEST_DIR/project/.vrift/manifest.lmdb"

# Behavior-based daemon check instead of pgrep
if ! "$CLI_PATH" daemon status 2>/dev/null | grep -q "running\|Operational"; then
    echo "[4] Starting daemon..."
    "$DAEMON_PATH" start &
    sleep 2
else
    echo "[4] Daemon already running (verified via behavior check)"
fi

# Test 1: Direct compile without shim (baseline)
echo ""
echo "[5] Baseline compile (no shim)..."
cd "$TEST_DIR/project"
if make clean && make; then
    echo "    ✅ Baseline compile succeeded"
    ./myprogram
    BASELINE_OK=true
else
    echo "    ❌ Baseline compile failed"
    BASELINE_OK=false
fi

# Test 2: Compile with shim
echo ""
echo "[6] Compile with VFS shim..."
make clean

export DYLD_INSERT_LIBRARIES="$SHIM_PATH"
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_MANIFEST="$MANIFEST_PATH"

cd "$TEST_DIR/project"
if make 2>&1; then
    echo "    ✅ VFS compile succeeded!"
    
    # Run the compiled program
    if ./myprogram; then
        echo "    ✅ Compiled program runs correctly"
        VFS_OK=true
    else
        echo "    ❌ Compiled program failed to run"
        VFS_OK=false
    fi
else
    echo "    ❌ VFS compile FAILED"
    echo ""
    echo "    This means compilers break under VFS!"
    echo "    Likely causes:"
    echo "    - unlink returns EROFS"
    echo "    - rename returns EROFS"
    echo "    - open(O_TRUNC) fails"
    VFS_OK=false
fi

# Cleanup
unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE
unset VRIFT_MANIFEST

echo ""
echo "[7] Cleanup..."
rm -rf "$TEST_DIR" 2>/dev/null || true

# Results
echo ""
echo "=== Results ==="
if [[ "$BASELINE_OK" == "true" ]] && [[ "$VFS_OK" == "true" ]]; then
    echo "✅ GCC compile works in VFS mode!"
    exit 0
elif [[ "$BASELINE_OK" == "true" ]] && [[ "$VFS_OK" == "false" ]]; then
    echo "❌ CRITICAL: GCC fails under VFS but works normally"
    echo "   This PROVES vrift breaks compilers!"
    exit 1
else
    echo "⚠️ Baseline compile failed, cannot test VFS"
    exit 1
fi
