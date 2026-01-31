#!/bin/bash
# VRift GC Demo Script
# One-click demonstration of the complete GC lifecycle
#
# Usage: ./scripts/demo_gc.sh [--skip-npm]
#
# Options:
#   --skip-npm    Skip npm install if node_modules already exists

set -e

VRIFT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="/tmp/vrift-gc-demo"
CAS_DIR="$DEMO_DIR/cas"
VRIFT="$VRIFT_ROOT/target/release/vrift"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

print_header() {
    echo ""
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}  $1${NC}"
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""
}

print_step() {
    echo -e "${YELLOW}▶ Step $1: $2${NC}"
    echo ""
}

print_success() {
    echo -e "${GREEN}✅ $1${NC}"
}

print_info() {
    echo -e "${BLUE}ℹ  $1${NC}"
}

# Parse arguments
SKIP_NPM=false
for arg in "$@"; do
    case $arg in
        --skip-npm)
            SKIP_NPM=true
            shift
            ;;
    esac
done

# Check vrift binary exists
if [ ! -f "$VRIFT" ]; then
    echo -e "${RED}Error: vrift binary not found at $VRIFT${NC}"
    echo "Run: cargo build --release"
    exit 1
fi

print_header "VRift GC Demo (RFC-0041)"

echo "Demo Directory: $DEMO_DIR"
echo "CAS Directory:  $CAS_DIR"
echo "VRift Binary:   $VRIFT"
echo ""

# ============================================================================
# Setup
# ============================================================================

print_step "0" "Setup - Creating demo environment"

rm -rf "$DEMO_DIR"
rm -f ~/.vrift/registry/manifests.json

mkdir -p "$DEMO_DIR/proj1" "$DEMO_DIR/proj2" "$CAS_DIR"

# Copy package.json from examples
cp "$VRIFT_ROOT/examples/benchmarks/medium_package.json" "$DEMO_DIR/proj1/package.json"
cp "$VRIFT_ROOT/examples/benchmarks/medium_package.json" "$DEMO_DIR/proj2/package.json"

print_success "Created demo directories"

# ============================================================================
# npm install (can be skipped)
# ============================================================================

if [ "$SKIP_NPM" = true ] && [ -d "$DEMO_DIR/proj1/node_modules" ]; then
    print_info "Skipping npm install (--skip-npm flag)"
else
    print_step "0.1" "Installing npm dependencies for proj1 (this may take a minute...)"
    cd "$DEMO_DIR/proj1"
    npm install --legacy-peer-deps --silent 2>&1 | tail -1
    PROJ1_FILES=$(find node_modules -type f 2>/dev/null | wc -l | tr -d ' ')
    print_success "proj1: $PROJ1_FILES files installed"

    print_step "0.2" "Installing npm dependencies for proj2"
    cd "$DEMO_DIR/proj2"
    npm install --legacy-peer-deps --silent 2>&1 | tail -1
    PROJ2_FILES=$(find node_modules -type f 2>/dev/null | wc -l | tr -d ' ')
    print_success "proj2: $PROJ2_FILES files installed"
fi

cd "$VRIFT_ROOT"

# ============================================================================
# Step 1: Ingest proj1
# ============================================================================

print_step "1" "Ingest proj1 (node_modules)"

"$VRIFT" --the-source-root "$CAS_DIR" ingest "$DEMO_DIR/proj1/node_modules" -o "$DEMO_DIR/proj1.manifest"

# ============================================================================
# Step 2: Ingest proj2 (expect high dedup!)
# ============================================================================

print_step "2" "Ingest proj2 (expect high dedup - same dependencies!)"

"$VRIFT" --the-source-root "$CAS_DIR" ingest "$DEMO_DIR/proj2/node_modules" -o "$DEMO_DIR/proj2.manifest"

# ============================================================================
# Step 3: GC Status (all healthy)
# ============================================================================

print_step "3" "GC Status (all healthy, 2 active manifests)"

"$VRIFT" --the-source-root "$CAS_DIR" gc

# ============================================================================
# Step 4: Delete proj1 manifest (simulate project deletion)
# ============================================================================

print_step "4" "Delete proj1.manifest (simulate project deletion)"

rm "$DEMO_DIR/proj1.manifest"
print_success "Deleted proj1.manifest"
echo ""

# ============================================================================
# Step 5: GC detects stale manifest
# ============================================================================

print_step "5" "GC now detects stale manifest and orphaned blobs"

"$VRIFT" --the-source-root "$CAS_DIR" gc

# ============================================================================
# Step 6: Prune stale + delete orphans
# ============================================================================

print_step "6" "GC --prune-stale --delete (cleanup orphans)"

"$VRIFT" --the-source-root "$CAS_DIR" gc --prune-stale --delete

# ============================================================================
# Step 7: Final status
# ============================================================================

print_step "7" "Final GC status (clean)"

"$VRIFT" --the-source-root "$CAS_DIR" gc

# ============================================================================
# Summary
# ============================================================================

print_header "Demo Complete!"

echo -e "${GREEN}The GC lifecycle demonstrated:${NC}"
echo "  1. Ingest multiple projects into shared CAS"
echo "  2. Cross-project deduplication (99%+ for same deps)"
echo "  3. Automatic manifest registry tracking"
echo "  4. Stale manifest detection when source deleted"
echo "  5. Orphan blob identification with size stats"
echo "  6. Safe cleanup with --prune-stale --delete"
echo ""
echo -e "${BLUE}To run again: ./scripts/demo_gc.sh${NC}"
echo -e "${BLUE}To skip npm:  ./scripts/demo_gc.sh --skip-npm${NC}"
echo ""
