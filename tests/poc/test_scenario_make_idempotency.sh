#!/bin/bash
# Scenario: Build System Idempotency (Make)
# Verifies that 'make' correctly identifies virtual file states.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR=$(mktemp -d)
# trap 'rm -rf "$WORK_DIR"' EXIT

VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

# 1. Setup Project
cd "$WORK_DIR"
mkdir project
cd project
"$VRIFT_BIN" init . >/dev/null

# Create a real main.c and Makefile
cat <<EOF > main.c
#include <stdio.h>
#include "header.h"
int main() {
    printf("V: %d\n", VERSION);
    return 0;
}
EOF

cat <<EOF > Makefile
main: main.c header.h
	gcc -O2 -o main main.c
EOF

# 2. Start Daemon and Ingest
echo "#define VERSION 1" > header.h
"$PROJECT_ROOT/target/release/vriftd" > "$WORK_DIR/vriftd.log" 2>&1 &
DAEMON_PID=$!
sleep 2 # Wait for daemon to start

"$VRIFT_BIN" ingest .

# 3. First Build
echo "🏗️ First Build..."
# Inception mode environment (we manually export for now to be precise)
export VRIFT_VFS_PREFIX="$WORK_DIR/project"
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

# Use local gcc copy to bypass SIP
cp /usr/bin/gcc "$WORK_DIR/gcc"
# Since inception exported CC might be system-protected, we override here
export CC="$WORK_DIR/gcc"

make 2>&1 | grep -v "vrift" || true

if [ -f main ]; then
    echo "✅ Success: First build complete."
else
    echo "❌ Failure: First build failed."
    "$VRIFT_BIN" daemon stop
    exit 1
fi

# 4. Check Idempotency (should say 'up to date')
echo "🔄 Checking idempotency (should be up to date)..."
OUT=$(make 2>&1)
if [[ "$OUT" == *"is up to date"* ]]; then
    echo "✅ Success: Make is idempotent."
else
    echo "❌ Failure: Make triggered redundant build."
    echo "   Output: $OUT"
fi

# 5. Trigger Rebuild
echo "📝 Changing file content and re-ingesting..."
sleep 1
echo "#define VERSION 2" > header.h
"$VRIFT_BIN" ingest .

echo "🔄 Checking rebuild..."
OUT=$(make 2>&1)
if [[ "$OUT" == *"gcc"* ]]; then
    echo "✅ Success: Rebuild triggered."
else
    echo "❌ Failure: Rebuild NOT triggered."
    echo "   Output: $OUT"
kill $DAEMON_PID 2>/dev/null || true
echo "🎉 Scenario Test Complete."
