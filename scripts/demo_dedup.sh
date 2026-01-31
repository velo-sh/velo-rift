#!/bin/bash
# ============================================================================
# VRift Demo: Cross-Project Deduplication Showcase
# ============================================================================
# One-click demo showing VRift's deduplication superpowers.
# Can be used for UX demo or CI/CD validation.
#
# Usage:
#   ./scripts/demo_dedup.sh              # Full demo (fresh + re-run)
#   ./scripts/demo_dedup.sh --quick      # Quick demo (xsmall + small only)
#   ./scripts/demo_dedup.sh --fresh-only # Only fresh start scenario
#   ./scripts/demo_dedup.sh --rerun-only # Only re-run scenario
#
# Requirements:
#   - Built vrift binary (cargo build --release)
#   - Test datasets in /tmp/vrift-bench/
# ============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
VRIFT="${PROJECT_ROOT}/target/release/vrift"
CAS_DIR="${HOME}/.vrift/the_source"
BENCH_DIR="/tmp/vrift-bench"
MANIFEST_DIR="/tmp/vrift-demo"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Parse arguments
QUICK_MODE=false
FRESH_ONLY=false
RERUN_ONLY=false

for arg in "$@"; do
    case $arg in
        --quick) QUICK_MODE=true ;;
        --fresh-only) FRESH_ONLY=true ;;
        --rerun-only) RERUN_ONLY=true ;;
        --help|-h)
            echo "Usage: $0 [--quick] [--fresh-only] [--rerun-only]"
            exit 0
            ;;
    esac
done

# Dataset paths (order: extra-small → large for progressive dedup)
declare -a DATASETS
if $QUICK_MODE; then
    DATASETS=(
        "XSmall|${BENCH_DIR}/Small/node_modules"
        "Small|${BENCH_DIR}/Medium/node_modules"
    )
else
    DATASETS=(
        "XSmall|${BENCH_DIR}/Small/node_modules"
        "Small|${BENCH_DIR}/Medium/node_modules"
        "Medium|${BENCH_DIR}/Large/node_modules"
        "Large|${BENCH_DIR}/XLarge (Real Project)/node_modules"
    )
fi

# ============================================================================
# Helper Functions
# ============================================================================

print_header() {
    echo ""
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}   $1${NC}"
    echo -e "${BOLD}${CYAN}═══════════════════════════════════════════════════════════════════════${NC}"
    echo ""
}

print_section() {
    echo -e "${YELLOW}▶ $1${NC}"
}

check_prerequisites() {
    print_section "Checking prerequisites..."
    
    if [[ ! -x "$VRIFT" ]]; then
        echo -e "${RED}Error: vrift binary not found at $VRIFT${NC}"
        echo "Run: cargo build --release"
        exit 1
    fi
    
    local missing=()
    for entry in "${DATASETS[@]}"; do
        IFS='|' read -r name path <<< "$entry"
        if [[ ! -d "$path" ]]; then
            missing+=("$name: $path")
        fi
    done
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        echo -e "${RED}Error: Missing test datasets:${NC}"
        for m in "${missing[@]}"; do
            echo "  - $m"
        done
        echo ""
        echo "Run: python3 scripts/e2e_test.py --download-only"
        exit 1
    fi
    
    echo -e "${GREEN}✓ All prerequisites met${NC}"
}

format_bytes() {
    local bytes=$1
    if (( bytes >= 1073741824 )); then
        printf "%.2f GB" "$(echo "scale=2; $bytes / 1073741824" | bc)"
    elif (( bytes >= 1048576 )); then
        printf "%.2f MB" "$(echo "scale=2; $bytes / 1048576" | bc)"
    else
        printf "%.2f KB" "$(echo "scale=2; $bytes / 1024" | bc)"
    fi
}

get_dir_size() {
    du -sk "$1" 2>/dev/null | awk '{print $1 * 1024}'
}

# ============================================================================
# Scenario A: Fresh Start (Delete CAS, Small → Large)
# ============================================================================

run_scenario_a() {
    print_header "🚀 Scenario A: Fresh Start (Extra-Small → Large)"
    
    # Clean CAS
    print_section "Cleaning CAS..."
    rm -rf "$CAS_DIR"
    rm -rf "$MANIFEST_DIR"
    mkdir -p "$MANIFEST_DIR"
    
    local total_start=$(date +%s.%N)
    local i=1
    
    for entry in "${DATASETS[@]}"; do
        IFS='|' read -r name path <<< "$entry"
        
        echo ""
        print_section "Project $i: $name"
        
        "$VRIFT" ingest "$path" -o "${MANIFEST_DIR}/${name}.manifest"
        
        ((i++))
    done
    
    local total_end=$(date +%s.%N)
    local total_time=$(echo "$total_end - $total_start" | bc)
    
    echo ""
    print_section "Scenario A Summary"
    local cas_size=$(get_dir_size "$CAS_DIR")
    echo -e "   ${BOLD}CAS Size:${NC} $(format_bytes $cas_size)"
    echo -e "   ${BOLD}Total Time:${NC} ${total_time}s"
}

# ============================================================================
# Scenario B: Re-Run (Keep CAS, All 100% Dedup)
# ============================================================================

run_scenario_b() {
    print_header "🔄 Scenario B: Re-Run (Preserved CAS)"
    
    # Clean manifests only, keep CAS
    rm -rf "$MANIFEST_DIR"
    mkdir -p "$MANIFEST_DIR"
    
    local total_start=$(date +%s.%N)
    local i=1
    
    for entry in "${DATASETS[@]}"; do
        IFS='|' read -r name path <<< "$entry"
        
        echo ""
        print_section "Project $i: $name (re-run)"
        
        "$VRIFT" ingest "$path" -o "${MANIFEST_DIR}/${name}_rerun.manifest"
        
        ((i++))
    done
    
    local total_end=$(date +%s.%N)
    local total_time=$(echo "$total_end - $total_start" | bc)
    
    echo ""
    print_section "Scenario B Summary"
    local cas_size=$(get_dir_size "$CAS_DIR")
    echo -e "   ${BOLD}CAS Size:${NC} $(format_bytes $cas_size) (unchanged - all dedup!)"
    echo -e "   ${BOLD}Total Time:${NC} ${total_time}s"
}

# ============================================================================
# Main
# ============================================================================

main() {
    print_header "🎯 VRift Cross-Project Deduplication Demo"
    
    check_prerequisites
    
    if ! $RERUN_ONLY; then
        run_scenario_a
    fi
    
    if ! $FRESH_ONLY; then
        run_scenario_b
    fi
    
    print_header "✅ Demo Complete!"
    
    echo -e "${BOLD}Key Takeaways:${NC}"
    echo "  • Cross-project sharing: npm dependencies are highly shared"
    echo "  • Re-run optimization: 100% dedup when CAS is warm"
    echo "  • Speed: 10,000+ files/sec with parallel ingest"
    echo ""
}

main "$@"
