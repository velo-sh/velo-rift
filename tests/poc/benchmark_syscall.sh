#!/bin/bash
# Performance Benchmark: Raw ASM Syscalls vs LibC
#
# Compares the overhead of using inline assembly syscalls directly
# versus going through libc wrapper functions.
#
# Theory: Raw syscalls should be faster because:
# 1. No function call overhead (direct svc/syscall instruction)
# 2. No libc wrapper code (parameter validation, errno, etc.)
# 3. No PLT indirection (dynamic linker costs)

set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Raw Syscall vs LibC Performance Benchmark ==="
echo ""

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

# Create benchmark program
BENCH_BIN="/tmp/bench_syscall"
cat > "${BENCH_BIN}.c" << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <time.h>

#define ITERATIONS 1000000

static inline long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int main(int argc, char **argv) {
    int mode = (argc > 1) ? atoi(argv[1]) : 0;
    int fd = open("/dev/null", O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }

    struct stat sb;
    long long start, end;
    int i;

    // Warm up
    for (i = 0; i < 1000; i++) {
        fstat(fd, &sb);
    }

    // Benchmark fstat
    start = now_ns();
    for (i = 0; i < ITERATIONS; i++) {
        fstat(fd, &sb);
    }
    end = now_ns();

    double ns_per_call = (double)(end - start) / ITERATIONS;
    printf("fstat: %.2f ns/call (%d iterations)\n", ns_per_call, ITERATIONS);

    close(fd);
    return 0;
}
EOF

cc -O2 -o "$BENCH_BIN" "${BENCH_BIN}.c"
rm -f "${BENCH_BIN}.c"

# Baseline: No shim (direct libc)
echo -e "${BLUE}[Baseline] Direct LibC (no shim):${NC}"
"$BENCH_BIN"
BASELINE_NS=$("$BENCH_BIN" | grep -oE '[0-9]+\.[0-9]+')

# With shim: Using raw syscall during init check
SHIM_PATH="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "Building shim..."
    cargo build --release -p vrift-inception-layer --manifest-path "$PROJECT_ROOT/Cargo.toml"
fi

codesign -f -s - "$BENCH_BIN" 2>/dev/null || true

echo ""
echo -e "${BLUE}[With Shim] LibC via shim interposition:${NC}"
# Note: Do NOT use DYLD_FORCE_FLAT_NAMESPACE as it causes hangs during dyld bootstrap
# Use a temporary file for results
SHIM_OUT=$(mktemp)
VRIFT_DEBUG=0 env DYLD_INSERT_LIBRARIES="$SHIM_PATH" "$BENCH_BIN" > "$SHIM_OUT" 2>&1 &
BENCH_PID=$!

# Wait for completion with a 15-second timeout
FINISHED=0
for i in $(seq 1 15); do
    if ! kill -0 $BENCH_PID 2>/dev/null; then
        FINISHED=1
        break
    fi
    sleep 1
done

if [[ $FINISHED -eq 0 ]]; then
    kill -9 $BENCH_PID 2>/dev/null
    echo "⚠️  Shim benchmark timed out"
    SHIM_NS="N/A"
else
    # The process finished, capture the last line
    cat "$SHIM_OUT"
    SHIM_NS=$(grep -oE '[0-9]+\.[0-9]+' "$SHIM_OUT" | tail -n 1)
fi
rm -f "$SHIM_OUT"

# Calculate overhead
echo ""
echo "=== Summary ==="
echo -e "Baseline (libc):      ${GREEN}${BASELINE_NS} ns/call${NC}"
echo -e "With shim:            ${GREEN}${SHIM_NS:-N/A} ns/call${NC}"

if [[ -n "$SHIM_NS" && "$SHIM_NS" != "N/A" ]] && command -v bc &> /dev/null; then
    OVERHEAD=$(echo "scale=2; (($SHIM_NS - $BASELINE_NS) / $BASELINE_NS) * 100" | bc 2>/dev/null || echo "N/A")
    echo -e "Overhead:             ${OVERHEAD}%"
fi

echo ""
echo "Note: The shim checks INITIALIZING state before calling real function."
echo "      Raw syscalls are only used during early init (INITIALIZING >= 2)."
echo "      After init completes, shim uses normal libc via dlsym-cached pointers."

# Cleanup
rm -f "$BENCH_BIN"
