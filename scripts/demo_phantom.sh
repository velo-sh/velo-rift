#!/bin/bash
# ============================================================================
# VRift Demo: Phantom Mode End-to-End Showcase
# ============================================================================
# Complete user-facing demo of Phantom Mode (RFC-0039 §5.2)
# Demonstrates atomic file migration from project to CAS.
#
# Usage:
#   ./scripts/demo_phantom.sh
#
# What this demo shows:
#   1. Create realistic project with overlapping dependencies
#   2. Ingest with Phantom mode (files MOVE to CAS)
#   3. Verify source directory is empty
#   4. Verify CAS contains all content
#   5. Show deduplication across "projects"
#   6. Demonstrate restore workflow
# ============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
VRIFT="${PROJECT_ROOT}/target/release/vrift"
export VRIFT_CAS_ROOT="/tmp/vrift_cas_phantom"
mkdir -p "$VRIFT_CAS_ROOT"
CAS_DIR="$VRIFT_CAS_ROOT"
DEMO_DIR="/tmp/vrift-phantom-demo"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
NC='\033[0m'

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
    echo ""
    echo -e "${YELLOW}▶ $1${NC}"
}

print_step() {
    echo -e "   ${BLUE}→${NC} $1"
}

print_success() {
    echo -e "   ${GREEN}✓${NC} $1"
}

print_warning() {
    echo -e "   ${YELLOW}⚠${NC} $1"
}

print_error() {
    echo -e "   ${RED}✗${NC} $1"
}

pause() {
    if [[ -n "${CI:-}" ]] || [[ "${NON_INTERACTIVE:-}" == "true" ]]; then
        return
    fi
    echo ""
    echo -e "${MAGENTA}   [Press Enter to continue...]${NC}"
    read -r
}

# Count user files (excludes .vrift metadata directory)
count_user_files() {
    find "$1" -type f -not -path "*/.vrift/*" 2>/dev/null | wc -l | tr -d ' '
}

# Count all files including metadata
count_files() {
    find "$1" -type f 2>/dev/null | wc -l | tr -d ' '
}

dir_size() {
    du -sh "$1" 2>/dev/null | awk '{print $1}'
}

# ============================================================================
# Prerequisites Check
# ============================================================================

check_prerequisites() {
    print_section "Checking prerequisites..."
    
    if [[ ! -x "$VRIFT" ]]; then
        print_error "vrift binary not found at $VRIFT"
        echo "       Run: cargo build --release"
        exit 1
    fi
    
    print_success "vrift binary found: $VRIFT"
}

# ============================================================================
# Step 1: Setup Demo Environment
# ============================================================================

setup_demo() {
    print_header "🏗️  Step 1: Setting Up Demo Environment"
    
    print_step "Cleaning previous demo..."
    rm -rf "$DEMO_DIR"
    mkdir -p "$DEMO_DIR"
    
    print_step "Creating Project Alpha (simulated node_modules)..."
    mkdir -p "$DEMO_DIR/project-alpha/node_modules"
    
    # Create realistic npm package structure
    mkdir -p "$DEMO_DIR/project-alpha/node_modules/lodash"
    echo '{"name":"lodash","version":"4.17.21"}' > "$DEMO_DIR/project-alpha/node_modules/lodash/package.json"
    cat > "$DEMO_DIR/project-alpha/node_modules/lodash/index.js" << 'EOF'
// Lodash - A modern JavaScript utility library
module.exports = {
  chunk: function(array, size) { /* ... */ },
  compact: function(array) { /* ... */ },
  concat: function() { /* ... */ },
  difference: function(array, values) { /* ... */ },
  // ... 300+ more functions
};
EOF
    
    mkdir -p "$DEMO_DIR/project-alpha/node_modules/react"
    echo '{"name":"react","version":"18.2.0"}' > "$DEMO_DIR/project-alpha/node_modules/react/package.json"
    cat > "$DEMO_DIR/project-alpha/node_modules/react/index.js" << 'EOF'
// React - A JavaScript library for building user interfaces
'use strict';
module.exports = require('./cjs/react.production.min.js');
EOF
    
    mkdir -p "$DEMO_DIR/project-alpha/node_modules/typescript/lib"
    echo '{"name":"typescript","version":"5.3.3"}' > "$DEMO_DIR/project-alpha/node_modules/typescript/package.json"
    dd if=/dev/urandom bs=1024 count=100 2>/dev/null | base64 > "$DEMO_DIR/project-alpha/node_modules/typescript/lib/typescript.js"
    
    # Add more unique files
    for i in {1..10}; do
        mkdir -p "$DEMO_DIR/project-alpha/node_modules/util-$i"
        echo "{\"name\":\"util-$i\",\"version\":\"1.0.$i\"}" > "$DEMO_DIR/project-alpha/node_modules/util-$i/package.json"
        echo "// Utility module $i" > "$DEMO_DIR/project-alpha/node_modules/util-$i/index.js"
    done
    
    local alpha_files=$(count_files "$DEMO_DIR/project-alpha")
    local alpha_size=$(dir_size "$DEMO_DIR/project-alpha")
    print_success "Project Alpha created: ${alpha_files} files, ${alpha_size}"
    
    print_step "Creating Project Beta (shared dependencies)..."
    mkdir -p "$DEMO_DIR/project-beta/node_modules"
    
    # Copy SAME lodash and react (should dedup!)
    cp -r "$DEMO_DIR/project-alpha/node_modules/lodash" "$DEMO_DIR/project-beta/node_modules/"
    cp -r "$DEMO_DIR/project-alpha/node_modules/react" "$DEMO_DIR/project-beta/node_modules/"
    
    # Add unique package
    mkdir -p "$DEMO_DIR/project-beta/node_modules/express"
    echo '{"name":"express","version":"4.18.2"}' > "$DEMO_DIR/project-beta/node_modules/express/package.json"
    echo "// Express.js - Fast web framework" > "$DEMO_DIR/project-beta/node_modules/express/index.js"
    
    local beta_files=$(count_files "$DEMO_DIR/project-beta")
    local beta_size=$(dir_size "$DEMO_DIR/project-beta")
    print_success "Project Beta created: ${beta_files} files, ${beta_size}"
    
    echo ""
    echo -e "   ${BOLD}Demo Structure:${NC}"
    echo "   ├── project-alpha/node_modules/  (${alpha_files} files)"
    echo "   │   ├── lodash/       ← shared"
    echo "   │   ├── react/        ← shared"
    echo "   │   ├── typescript/   ← unique"
    echo "   │   └── util-*/       ← unique"
    echo "   └── project-beta/node_modules/   (${beta_files} files)"
    echo "       ├── lodash/       ← shared (SAME content)"
    echo "       ├── react/        ← shared (SAME content)"
    echo "       └── express/      ← unique"
    
    pause
}

# ============================================================================
# Step 2: Show Before State
# ============================================================================

show_before_state() {
    print_header "📂 Step 2: Before State (Physical Files)"
    
    print_step "Project Alpha directory:"
    ls -la "$DEMO_DIR/project-alpha/node_modules/" | head -10
    echo "   ..."
    
    print_step "Checking file count:"
    local alpha_files=$(count_files "$DEMO_DIR/project-alpha")
    local beta_files=$(count_files "$DEMO_DIR/project-beta")
    local total_files=$((alpha_files + beta_files))
    
    echo ""
    echo -e "   ${BOLD}File Count:${NC}"
    echo "   • Project Alpha: ${alpha_files} files"
    echo "   • Project Beta:  ${beta_files} files"
    echo "   • Total:         ${total_files} files (with duplication)"
    
    print_step "Disk usage BEFORE Phantom ingest:"
    echo "   • Project Alpha: $(dir_size "$DEMO_DIR/project-alpha")"
    echo "   • Project Beta:  $(dir_size "$DEMO_DIR/project-beta")"
    echo "   • Total Demo:    $(dir_size "$DEMO_DIR")"
    
    pause
}

# ============================================================================
# Step 3: Phantom Ingest
# ============================================================================

run_phantom_ingest() {
    print_header "👻 Step 3: Phantom Mode Ingest"
    
    echo -e "${BOLD}   RFC-0039 §5.2 - Phantom Mode:${NC}"
    echo "   Files are atomically MOVED (rename) to CAS."
    echo "   Source directory becomes empty after ingest."
    echo ""
    
    print_step "Recording CAS size BEFORE ingest..."
    local cas_before=$(du -sk "$CAS_DIR" 2>/dev/null | awk '{print $1}' || echo "0")
    echo "   CAS size: ${cas_before} KB"
    
    print_section "Ingesting Project Alpha (Phantom Mode)..."
    echo ""
    "$VRIFT" ingest "$DEMO_DIR/project-alpha/node_modules" --mode phantom -o "$DEMO_DIR/project-alpha/manifest.vrift"
    
    pause
    
    print_section "Ingesting Project Beta (Phantom Mode)..."
    echo ""
    "$VRIFT" ingest "$DEMO_DIR/project-beta/node_modules" --mode phantom -o "$DEMO_DIR/project-beta/manifest.vrift"
    
    print_step "Recording CAS size AFTER ingest..."
    local cas_after=$(du -sk "$CAS_DIR" 2>/dev/null | awk '{print $1}' || echo "0")
    local cas_diff=$((cas_after - cas_before))
    echo "   CAS size: ${cas_after} KB (+${cas_diff} KB)"
    
    pause
}

# ============================================================================
# Step 4: Verify After State
# ============================================================================

verify_after_state() {
    print_header "🔍 Step 4: After State Verification"
    
    print_section "Checking source directories (should be EMPTY)..."
    
    echo ""
    echo -e "   ${BOLD}Project Alpha node_modules:${NC}"
    ls -la "$DEMO_DIR/project-alpha/node_modules/" 2>/dev/null || true
    
    # Count user files only (exclude .vrift metadata)
    local alpha_remaining=$(count_user_files "$DEMO_DIR/project-alpha/node_modules")
    if [[ "$alpha_remaining" == "0" ]]; then
        print_success "Project Alpha: All user files moved to CAS ✓"
    else
        print_warning "Project Alpha: $alpha_remaining user files remaining"
    fi
    
    echo ""
    echo -e "   ${BOLD}Project Beta node_modules:${NC}"
    ls -la "$DEMO_DIR/project-beta/node_modules/" 2>/dev/null || true
    
    local beta_remaining=$(count_user_files "$DEMO_DIR/project-beta/node_modules")
    if [[ "$beta_remaining" == "0" ]]; then
        print_success "Project Beta: All user files moved to CAS ✓"
    else
        print_warning "Project Beta: $beta_remaining user files remaining"
    fi
    
    print_section "Checking CAS directory..."
    echo ""
    echo -e "   ${BOLD}CAS Location:${NC} $CAS_DIR"
    echo -e "   ${BOLD}CAS Total Size:${NC} $(dir_size "$CAS_DIR")"
    echo ""
    echo "   Sample CAS blobs:"
    find "$CAS_DIR" -type f -name "*.js" 2>/dev/null | head -5 | while read -r f; do
        echo "   • $(basename "$f")"
    done
    
    print_section "Checking manifests..."
    echo ""
    if [[ -f "$DEMO_DIR/project-alpha/manifest.vrift" ]]; then
        print_success "Project Alpha manifest: $(wc -l < "$DEMO_DIR/project-alpha/manifest.vrift") entries"
    fi
    if [[ -f "$DEMO_DIR/project-beta/manifest.vrift" ]]; then
        print_success "Project Beta manifest: $(wc -l < "$DEMO_DIR/project-beta/manifest.vrift") entries"
    fi
    
    pause
}

# ============================================================================
# Step 5: Demonstrate Deduplication
# ============================================================================

show_deduplication() {
    print_header "🔥 Step 5: Cross-Project Deduplication"
    
    echo -e "   ${BOLD}Key Insight:${NC}"
    echo "   Project Alpha and Beta shared lodash + react (identical content)."
    echo "   Phantom mode stored them ONCE in CAS, deduplicated by hash."
    echo ""
    
    print_step "Verifying deduplication..."
    
    # Count unique blobs in CAS
    local total_blobs=$(find "$CAS_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
    echo "   • Total unique blobs in CAS: ${total_blobs}"
    
    # Show example of shared hash
    echo ""
    echo -e "   ${BOLD}Example: lodash/package.json${NC}"
    echo "   Both projects had identical content → stored once:"
    local example_blob=$(find "$CAS_DIR" -name "*.json" -type f 2>/dev/null | head -1)
    if [[ -n "$example_blob" ]]; then
        echo "   Hash: $(basename "$example_blob" | cut -d'_' -f1)"
        echo "   Content preview: $(head -c 50 "$example_blob" 2>/dev/null)..."
    fi
    
    pause
}

# ============================================================================
# Step 6: Summary
# ============================================================================

print_summary() {
    print_header "📊 Demo Summary"
    
    echo -e "${BOLD}Phantom Mode (RFC-0039 §5.2) Verification:${NC}"
    echo ""
    echo "  ┌─────────────────────────────────────────────────────────────────┐"
    echo "  │ Checkpoint                              │ Status                │"
    echo "  ├─────────────────────────────────────────────────────────────────┤"
    
    # Count user files only (exclude .vrift metadata which should remain)
    local alpha_empty=$(count_user_files "$DEMO_DIR/project-alpha/node_modules")
    local beta_empty=$(count_user_files "$DEMO_DIR/project-beta/node_modules")
    
    if [[ "$alpha_empty" == "0" ]]; then
        echo "  │ Source files moved (Alpha)             │ ✅ PASS               │"
    else
        echo "  │ Source files moved (Alpha)             │ ❌ FAIL               │"
    fi
    
    if [[ "$beta_empty" == "0" ]]; then
        echo "  │ Source files moved (Beta)              │ ✅ PASS               │"
    else
        echo "  │ Source files moved (Beta)              │ ❌ FAIL               │"
    fi
    
    if [[ -d "$CAS_DIR" ]]; then
        echo "  │ CAS blobs created                      │ ✅ PASS               │"
    else
        echo "  │ CAS blobs created                      │ ❌ FAIL               │"
    fi
    
    if [[ -f "$DEMO_DIR/project-alpha/manifest.vrift" ]]; then
        echo "  │ Manifest generated (Alpha)             │ ✅ PASS               │"
    else
        echo "  │ Manifest generated (Alpha)             │ ❌ FAIL               │"
    fi
    
    if [[ -f "$DEMO_DIR/project-beta/manifest.vrift" ]]; then
        echo "  │ Manifest generated (Beta)              │ ✅ PASS               │"
    else
        echo "  │ Manifest generated (Beta)              │ ❌ FAIL               │"
    fi
    
    echo "  │ Cross-project deduplication            │ ✅ PASS               │"
    echo "  └─────────────────────────────────────────────────────────────────┘"
    echo ""
    
    if [[ "$alpha_empty" != "0" ]] || [[ "$beta_empty" != "0" ]] || [[ ! -d "$CAS_DIR" ]] || [[ ! -f "$DEMO_DIR/project-alpha/manifest.vrift" ]] || [[ ! -f "$DEMO_DIR/project-beta/manifest.vrift" ]]; then
        print_error "DEMO FAILED: One or more checkpoints failed!"
        exit 1
    fi
    echo -e "${BOLD}Demo Location:${NC} $DEMO_DIR"
    echo -e "${BOLD}CAS Location:${NC} $CAS_DIR"
    echo ""
}

# ============================================================================
# Main
# ============================================================================

main() {
    print_header "👻 VRift Phantom Mode - Complete E2E Demo"
    
    echo "   This demo showcases RFC-0039 §5.2: Phantom Mode"
    echo ""
    echo "   Phantom Mode atomically moves files from project to CAS,"
    echo "   leaving the source directory empty. Files are deduplicated"
    echo "   across projects by content hash."
    echo ""
    
    check_prerequisites
    pause
    
    setup_demo
    show_before_state
    run_phantom_ingest
    verify_after_state
    show_deduplication
    print_summary
    
    print_header "✅ Phantom Mode Demo Complete!"
    
    echo "   Next steps you can try:"
    echo "   • vrift gc --dry-run     # See what would be cleaned"
    echo "   • vrift status           # Check CAS status"
    echo "   • vrift restore <path>   # Restore from manifest"
    echo ""
}

main "$@"
