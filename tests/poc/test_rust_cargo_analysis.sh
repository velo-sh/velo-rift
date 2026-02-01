#!/bin/bash
# Test: Rust Build - Analyze cargo's filesystem dependencies
# Goal: Trace what syscalls cargo/rustc actually use during a build
# This is an analysis test, not a pass/fail test

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== Rust Build Analysis: cargo syscall dependencies ==="
echo ""

# Create a simple test crate
rm -rf /tmp/cargo_analysis_crate
mkdir -p /tmp/cargo_analysis_crate/src

cat > /tmp/cargo_analysis_crate/Cargo.toml << 'EOF'
[package]
name = "analysis_crate"
version = "0.1.0"
edition = "2021"
EOF

cat > /tmp/cargo_analysis_crate/src/main.rs << 'EOF'
fn main() { println!("test"); }
EOF

echo "[1] Clean build - fingerprint generation"
rm -rf /tmp/cargo_analysis_crate/target
(cd /tmp/cargo_analysis_crate && cargo build 2>&1) > /dev/null

echo "[2] Analyze fingerprint structure"
echo ""
FINGERPRINT_DIR="/tmp/cargo_analysis_crate/target/debug/.fingerprint"
if [ -d "$FINGERPRINT_DIR" ]; then
    echo "Fingerprint directories:"
    ls "$FINGERPRINT_DIR" | head -10
    
    # Find the analysis_crate fingerprint
    CRATE_FP=$(ls "$FINGERPRINT_DIR" | grep "analysis_crate" | head -1)
    if [ -n "$CRATE_FP" ]; then
        echo ""
        echo "Fingerprint contents for analysis_crate:"
        ls "$FINGERPRINT_DIR/$CRATE_FP"
        
        echo ""
        echo "Fingerprint JSON:"
        cat "$FINGERPRINT_DIR/$CRATE_FP"/*.json 2>/dev/null | python3 -m json.tool 2>/dev/null | head -30
        
        echo ""
        echo "Dep-info (source file list):"
        cat "$FINGERPRINT_DIR/$CRATE_FP"/dep-* 2>/dev/null | strings | head -10
    fi
fi

echo ""
echo "[3] Incremental cache structure"
INCR_DIR="/tmp/cargo_analysis_crate/target/debug/incremental"
if [ -d "$INCR_DIR" ]; then
    echo "Incremental directories:"
    ls "$INCR_DIR"
    
    # Check session directory
    SESSION=$(ls "$INCR_DIR/analysis_crate-"*/s-* 2>/dev/null | head -1)
    if [ -n "$SESSION" ]; then
        echo ""
        echo "Session contents:"
        ls -la "$(dirname "$SESSION")"
    fi
fi

echo ""
echo "[4] Key observations for VFS:"
echo "  - cargo uses .fingerprint/ for mtime-based dirty detection"
echo "  - dep-info files list all source file paths (stat required)"
echo "  - invoked.timestamp is compared against source mtimes"
echo "  - incremental/ contains rustc query cache (can be large)"

# Cleanup
rm -rf /tmp/cargo_analysis_crate
