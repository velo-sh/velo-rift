#!/bin/bash
# Vrift Parallel Ingest Benchmark
#
# Demonstrates real-world zero-copy ingest performance.
# Reports: files, size, time, throughput.
#
# Usage:
#   ./scripts/benchmark_parallel.sh [--size small|medium|large|xlarge|all]
#
# // turbo-all

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BENCH_DIR="/tmp/vrift-bench"

SIZE="${1:-all}"
[[ "$SIZE" == "--size" ]] && SIZE="${2:-all}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${GREEN}=== Vrift Zero-Copy Ingest Benchmark ===${NC}"
echo ""

cargo build --release -p vrift-cli --quiet
VRIFT="$PROJECT_ROOT/target/release/vrift"

run_benchmark() {
    local name=$1
    local package_json=$2
    local work_dir="$BENCH_DIR/$name"
    
    echo -e "\n${YELLOW}━━━ $name ━━━${NC}"
    
    mkdir -p "$work_dir"
    
    # Only reinstall if package.json changed or node_modules missing
    if [[ ! -d "$work_dir/node_modules" ]] || ! diff -q "$package_json" "$work_dir/package.json" >/dev/null 2>&1; then
        cp "$package_json" "$work_dir/package.json"
        echo "Installing dependencies..."
        cd "$work_dir"
        npm install --silent --legacy-peer-deps 2>/dev/null || npm install --silent 2>/dev/null
    else
        echo "(Using cached node_modules)"
        cd "$work_dir"
    fi
    
    FILE_COUNT=$(find node_modules -type f 2>/dev/null | wc -l | tr -d ' ')
    DIR_COUNT=$(find node_modules -type d 2>/dev/null | wc -l | tr -d ' ')
    SIZE_KB=$(du -sk node_modules 2>/dev/null | awk '{print $1}')
    SIZE_MB=$(echo "scale=1; $SIZE_KB / 1024" | bc)
    
    echo -e "${CYAN}Files:${NC}  $FILE_COUNT  |  ${CYAN}Dirs:${NC} $DIR_COUNT  |  ${CYAN}Size:${NC} ${SIZE_MB}MB"
    
    rm -rf node_modules/.vrift 2>/dev/null || true
    CAS=$(mktemp -d)
    
    echo -n "Ingest: "
    START=$(python3 -c "import time; print(time.time())")
    "$VRIFT" --cas-root "$CAS" ingest node_modules -o /tmp/m.bin >/dev/null 2>&1
    END=$(python3 -c "import time; print(time.time())")
    
    TIME=$(python3 -c "print(f'{$END - $START:.2f}s')")
    RATE=$(python3 -c "print(f'{int($FILE_COUNT / ($END - $START)):,}')")
    
    echo -e "${GREEN}$TIME${NC} ($RATE files/sec)"
    
    rm -rf "$CAS"
}

case "$SIZE" in
    small)  run_benchmark "Small" "$PROJECT_ROOT/examples/benchmarks/small_package.json" ;;
    medium) run_benchmark "Medium" "$PROJECT_ROOT/examples/benchmarks/medium_package.json" ;;
    large)  run_benchmark "Large" "$PROJECT_ROOT/examples/benchmarks/large_package.json" ;;
    xlarge) run_benchmark "XLarge (Real Project)" "$PROJECT_ROOT/examples/benchmarks/xlarge_package.json" ;;
    all)
        run_benchmark "Small" "$PROJECT_ROOT/examples/benchmarks/small_package.json"
        run_benchmark "Medium" "$PROJECT_ROOT/examples/benchmarks/medium_package.json"
        run_benchmark "Large" "$PROJECT_ROOT/examples/benchmarks/large_package.json"
        run_benchmark "XLarge (Real Project)" "$PROJECT_ROOT/examples/benchmarks/xlarge_package.json"
        ;;
    *)
        echo "Usage: $0 [--size small|medium|large|xlarge|all]"
        exit 1
        ;;
esac

echo -e "\n${GREEN}=== Complete ===${NC}"
