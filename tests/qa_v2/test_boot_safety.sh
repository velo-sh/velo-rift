#!/bin/bash
set -e

# test_boot_safety.sh
# Verifies that the Velo Rift shim can safely handle high-frequency syscalls
# (stat, access, readlink) during dyld bootstrap without deadlocks.

WORK_DIR=$(mktemp -d -t vrift_boot_safety_XXXXXX)
# Use realpath if available to get absolute path
WORK_DIR=$(cd "$WORK_DIR" && pwd)

echo "📂 Work Directory: $WORK_DIR"

# Paths
SHIM_LIB="$(pwd)/target/release/libvrift_shim.dylib"

if [ ! -f "$SHIM_LIB" ]; then
    echo "❌ Error: Shim not found at $SHIM_LIB. Build it first with 'cargo build --release'."
    exit 1
fi

# 1. Create a dummy dylib that triggers many stat/readlink calls
cat <<EOF > "$WORK_DIR/hammer.c"
#include <unistd.h>
#include <sys/stat.h>
#include <stdio.h>

__attribute__((constructor))
void hammer_init() {
    struct stat st;
    char buf[1024];
    // Trigger calls that would previously deadlock or recurse
    stat("/usr/lib/libc.dylib", &st);
    access("/etc/passwd", R_OK);
    readlink("/var/run", buf, sizeof(buf));
    // fprintf(stderr, "[HAMMER] Safe boot calls completed\n");
}
EOF

# 2. Compile dylibs and an executable with dependencies
clang -dynamiclib "$WORK_DIR/hammer.c" -o "$WORK_DIR/libhammer.dylib"
cat <<EOF > "$WORK_DIR/main.c"
#include <stdio.h>
int main() {
    printf("[MAIN] Process started successfully\n");
    return 0;
}
EOF
clang "$WORK_DIR/main.c" -L"$WORK_DIR" -lhammer -o "$WORK_DIR/test_app"

# 3. Create a manifest for vrift-shim (even if empty)
export VRIFT_MANIFEST="$WORK_DIR/manifest.vdir"
echo "{\"files\": []}" > "$VRIFT_MANIFEST"

echo "🧪 Running boot safety stress test..."
# Run with timeout to detect hangs
# Using timeout command or a loop
COUNT=0
MAX=5
TIMEOUT_PED=5

while [ $COUNT -lt $MAX ]; do
    COUNT=$((COUNT + 1))
    echo "   Iteration $COUNT/$MAX..."
    
    # Run the test app with the shim injected
    # Use DYLD_FORCE_FLAT_NAMESPACE=1 to exacerbate issues
    (
        export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
        export DYLD_FORCE_FLAT_NAMESPACE=1
        export DYLD_LIBRARY_PATH="$WORK_DIR"
        export VRIFT_MANIFEST="$VRIFT_MANIFEST"
        export DYLD_PRINT_LIBRARIES=1
        export DYLD_PRINT_INTERPOSING=1
        "$WORK_DIR/test_app" > "$WORK_DIR/out.$COUNT" 2>&1
    ) &
    PID=$!
    
    # Wait for completion or timeout
    ELAPSED=0
    while kill -0 $PID 2>/dev/null; do
        if [ $ELAPSED -ge $TIMEOUT_PED ]; then
            echo "   ❌ Iteration $COUNT TIMED OUT (likely deadlock)!"
            kill -9 $PID 2>/dev/null || true
            exit 1
        fi
        sleep 1
        ELAPSED=$((ELAPSED + 1))
    done
    
    RESULT=$(cat "$WORK_DIR/out.$COUNT")
    if [[ "$RESULT" == *"[MAIN] Process started successfully"* ]]; then
        echo "   ✅ Iteration $COUNT success."
    else
        echo "   ❌ Iteration $COUNT failed."
        echo "   Output: $RESULT"
        exit 1
    fi
done

echo "----------------------------------------------------------------"
echo "🏆 BOOT SAFETY PROOF: SUCCESSFUL"
echo "----------------------------------------------------------------"

# Only cleanup on success
rm -rf "$WORK_DIR"
