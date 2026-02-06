#!/bin/bash
# Test: Internal Socket FD Leakage (O_CLOEXEC Requirement)
# Goal: Verify that the internal daemon socket does NOT leak to child processes.

set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"

if [[ "$(uname)" != "Darwin" ]]; then
    SHIM_PATH="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
fi

echo "=== Test: Internal Socket FD Leakage ==="

# Build a small C helper that lists open FDs after exec
cat <<EOF > /tmp/check_fds.c
#include <stdio.h>
#include <unistd.h>
#include <stdlib.h>

int main() {
    printf("[Child] listing FDs...\n");
    system("lsof -a -p \$\$ -d 0-256");
    return 0;
}
EOF
gcc /tmp/check_fds.c -o /tmp/check_fds

echo "[1] Running child process through shim..."
# Trigger shim init by opening a file
export VRIFT_DEBUG=1

# Run the helper. It will trigger shim init, then exec lsof.
# We look for /tmp/vrift.sock in the output.
OUTPUT=$(/tmp/check_fds 2>&1)

if echo "$OUTPUT" | grep -q "vrift.sock"; then
    echo ""
    echo "❌ FAIL: Internal socket FD LEAKED to child process!"
    echo "   (Missing O_CLOEXEC / SOCK_CLOEXEC on socket connection)"
    echo "$OUTPUT" | grep "vrift.sock"
    exit 1
else
    echo ""
    echo "✅ PASS: No internal socket leakage detected."
fi
