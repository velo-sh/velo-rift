#!/bin/bash
# ==============================================================================
# bench_inception.sh — Inception Layer Performance Benchmark
#
# Measures the overhead and acceleration of the VFS inception shim across
# multiple build scenarios. By default benchmarks the velo project; pass
# arguments to benchmark any Rust project with an active VDir.
#
# Usage:
#   ./scripts/bench_inception.sh                        # defaults to ../velo
#   ./scripts/bench_inception.sh /path/to/project       # custom project
#   ./scripts/bench_inception.sh /path/to/project src/lib.rs  # custom touch file
#
# Requirements:
#   - Release build of libvrift_inception_layer.dylib (or .so)
#   - A running vriftd daemon with a populated VDir for the target project
#   - The target project must have been ingested (vrift ingest)
#
# What it measures:
#   1. Baseline no-op     — cargo build without inception (warm cache)
#   2. Inception no-op    — cargo build with inception shim (measures shim overhead)
#   3. Touch incremental  — touch a source file, rebuild (incremental + shim)
#   4. Code incremental   — actual code change, rebuild (incremental + shim)
#   5. Source read stress  — stat+read all source files via inception (CAS accel)
#
# ==============================================================================
set -euo pipefail

# ── Configuration ─────────────────────────────────────────────────────────────

PROJECT_DIR="${1:-/Users/antigravity/rust_source/velo}"
TOUCH_FILE="${2:-}"
ITERATIONS="${BENCH_ITERATIONS:-3}"

# Resolve project dir to absolute path
PROJECT_DIR="$(cd "$PROJECT_DIR" && pwd -P)"

# Source SSOT env vars
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RIFT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export REPO_ROOT="$RIFT_ROOT"
source "$RIFT_ROOT/tests/lib/vrift_env.sh"

if [[ "$(uname)" == "Darwin" ]]; then
    SHIM_NAME="libvrift_inception_layer.dylib"
    DYLD_VAR="DYLD_INSERT_LIBRARIES"
    DYLD_FLAT="DYLD_FORCE_FLAT_NAMESPACE"
else
    SHIM_NAME="libvrift_inception_layer.so"
    DYLD_VAR="LD_PRELOAD"
    DYLD_FLAT=""
fi

# Search order: release > debug > project-local
SHIM=""
for candidate in \
    "$RIFT_ROOT/target/release/$SHIM_NAME" \
    "$RIFT_ROOT/target/debug/$SHIM_NAME" \
    "$PROJECT_DIR/.vrift/$SHIM_NAME"; do
    if [[ -f "$candidate" ]]; then
        SHIM="$candidate"
        break
    fi
done

if [[ -z "$SHIM" ]]; then
    echo "ERROR: Cannot find $SHIM_NAME. Build with: cargo build --release -p vrift-inception-layer"
    exit 1
fi

# Auto-detect VDir mmap path from project ID
# The Rust shim uses blake3(project_root)[..8].to_hex() = 16 hex chars
VDIR=""
if command -v b3sum &>/dev/null; then
    PROJECT_ID=$(echo -n "$PROJECT_DIR" | b3sum --no-names | head -c 16)
    CANDIDATE="$HOME/.vrift/vdir/${PROJECT_ID}.vdir"
    if [[ -f "$CANDIDATE" ]]; then
        VDIR="$CANDIDATE"
    fi
fi

# Fallback: use the vrift binary to derive the VDir path
if [[ -z "$VDIR" ]]; then
    VRIFT_BIN="$RIFT_ROOT/target/release/vrift"
    if [[ -x "$VRIFT_BIN" ]]; then
        VDIR=$($VRIFT_BIN config get vdir_path 2>/dev/null || echo "")
    fi
fi

# Fallback: find most recently modified .vdir file
if [[ -z "$VDIR" ]] && [[ -d "$HOME/.vrift/vdir" ]]; then
    VDIR=$(ls -t "$HOME/.vrift/vdir/"*.vdir 2>/dev/null | head -1 || echo "")
fi

# Auto-detect the source file to touch
if [[ -z "$TOUCH_FILE" ]]; then
    # Try common locations first
    for candidate in "src/lib.rs" "src/main.rs"; do
        if [[ -f "$PROJECT_DIR/$candidate" ]]; then
            TOUCH_FILE="$candidate"
            break
        fi
    done
    # Fallback: find a lib.rs in any crate, convert to relative path
    if [[ -z "$TOUCH_FILE" ]]; then
        ABS_PATH=$(find "$PROJECT_DIR" -maxdepth 4 -name 'lib.rs' -path '*/src/*' \
            -not -path '*/target/*' 2>/dev/null | head -1)
        if [[ -n "$ABS_PATH" ]]; then
            TOUCH_FILE="${ABS_PATH#$PROJECT_DIR/}"
        fi
    fi
fi

if [[ -z "$TOUCH_FILE" ]] || [[ ! -f "$PROJECT_DIR/$TOUCH_FILE" ]]; then
    echo "WARNING: No source file found to touch. Touch/Code-change tests will be skipped."
    TOUCH_FILE=""
fi

# Socket path (from SSOT helper)
SOCK="$VRIFT_SOCKET_PATH"

# CAS root (from SSOT helper)
CAS_ROOT="$VR_THE_SOURCE"

# ── Helpers ───────────────────────────────────────────────────────────────────

ms() { python3 -c 'import time; print(int(time.time()*1000))'; }

INCEP() {
    local env_args=(
        "VRIFT_PROJECT_ROOT=$PROJECT_DIR"
        "VRIFT_VFS_PREFIX=$PROJECT_DIR"
        "VRIFT_SOCKET_PATH=$SOCK"
        "VR_THE_SOURCE=$CAS_ROOT"
        "VRIFT_INCEPTION=1"
    )

    if [[ -n "$VDIR" ]]; then
        env_args+=("VRIFT_VDIR_MMAP=$VDIR")
    fi

    env_args+=("$DYLD_VAR=$SHIM")
    if [[ -n "$DYLD_FLAT" ]]; then
        env_args+=("$DYLD_FLAT=1")
    fi

    env "${env_args[@]}" "$@"
}

run_bench() {
    local label="$1"
    shift
    local times=()

    echo ""
    echo "── $label ──"
    for i in $(seq 1 "$ITERATIONS"); do
        T0=$(ms)
        eval "$@"
        T1=$(ms)
        local elapsed=$((T1 - T0))
        times+=("$elapsed")
        echo "  Run $i: ${elapsed}ms"
    done

    # Calculate average
    local sum=0
    for t in "${times[@]}"; do sum=$((sum + t)); done
    local avg=$((sum / ITERATIONS))
    echo "  Avg: ${avg}ms"
}

# ── Main ──────────────────────────────────────────────────────────────────────

echo ""
echo "╔═══════════════════════════════════════════════════╗"
echo "║  Inception Layer Benchmark                       ║"
echo "╚═══════════════════════════════════════════════════╝"
echo ""
echo "  Project:    $PROJECT_DIR"
echo "  Shim:       $SHIM"
echo "  VDir:       ${VDIR:-NONE (no CAS acceleration)}"
echo "  Socket:     $SOCK"
echo "  Touch file: ${TOUCH_FILE:-NONE}"
echo "  Iterations: $ITERATIONS"
echo ""

cd "$PROJECT_DIR"

# Warmup — ensure cargo index is cached
echo "Warming up..."
cargo build >/dev/null 2>&1 || true
echo ""

# ── 1) Baseline no-op ────────────────────────────────────────────────────────
run_bench "Baseline no-op (no inception)" \
    'cargo build >/dev/null 2>&1 || true'

# ── 2) Inception no-op ───────────────────────────────────────────────────────
run_bench "Inception no-op (shim loaded)" \
    'INCEP cargo build >/dev/null 2>&1 || true'

# ── 3) Touch incremental (baseline vs inception) ─────────────────────────────
if [[ -n "$TOUCH_FILE" ]]; then
    echo ""
    echo "── Touch incremental (BASELINE) ──"
    for i in $(seq 1 "$ITERATIONS"); do
        sleep 1; touch "$TOUCH_FILE"
        T0=$(ms); cargo build >/dev/null 2>&1 || true; T1=$(ms)
        echo "  Run $i: $((T1 - T0))ms"
    done

    echo ""
    echo "── Touch incremental (INCEPTION) ──"
    for i in $(seq 1 "$ITERATIONS"); do
        sleep 1; touch "$TOUCH_FILE"
        T0=$(ms); INCEP cargo build >/dev/null 2>&1 || true; T1=$(ms)
        echo "  Run $i: $((T1 - T0))ms"
    done
fi

# ── 4) Code change incremental (baseline vs inception) ───────────────────────
if [[ -n "$TOUCH_FILE" ]]; then
    cp "$TOUCH_FILE" "${TOUCH_FILE}.bench_bak"

    echo ""
    echo "── Code change incremental (BASELINE) ──"
    for i in $(seq 1 "$ITERATIONS"); do
        echo "" >> "$TOUCH_FILE"
        echo "// bench_canary_${i}_$(date +%s)" >> "$TOUCH_FILE"
        T0=$(ms); cargo build >/dev/null 2>&1 || true; T1=$(ms)
        echo "  Run $i: $((T1 - T0))ms"
        cp "${TOUCH_FILE}.bench_bak" "$TOUCH_FILE"
        cargo build >/dev/null 2>&1 || true  # restore baseline
    done

    echo ""
    echo "── Code change incremental (INCEPTION) ──"
    for i in $(seq 1 "$ITERATIONS"); do
        echo "" >> "$TOUCH_FILE"
        echo "// bench_canary_${i}_$(date +%s)" >> "$TOUCH_FILE"
        T0=$(ms); INCEP cargo build >/dev/null 2>&1 || true; T1=$(ms)
        echo "  Run $i: $((T1 - T0))ms"
        cp "${TOUCH_FILE}.bench_bak" "$TOUCH_FILE"
        INCEP cargo build >/dev/null 2>&1 || true  # restore baseline
    done

    rm -f "${TOUCH_FILE}.bench_bak"
fi

# ── 5) Source read stress (CAS acceleration test) ─────────────────────────────
echo ""
echo "── Source file stat stress (CAS acceleration) ──"
SRC_COUNT=$(find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' | wc -l | tr -d ' ')
echo "  Source files: $SRC_COUNT .rs files"

# Baseline: stat all source files without inception
T0=$(ms)
find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' -exec stat {} + >/dev/null 2>&1
T1=$(ms)
echo "  Baseline stat-all: $((T1 - T0))ms"

# Inception: stat all source files with inception (should use VDir)
T0=$(ms)
INCEP find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' -exec stat {} + >/dev/null 2>&1 || true
T1=$(ms)
echo "  Inception stat-all: $((T1 - T0))ms"

# Baseline: cat all source files (read content)
T0=$(ms)
find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' -exec cat {} + >/dev/null 2>&1 || true
T1=$(ms)
echo "  Baseline read-all: $((T1 - T0))ms"

# Inception: cat all source files (should serve from CAS)
T0=$(ms)
INCEP find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' -exec cat {} + >/dev/null 2>&1 || true
T1=$(ms)
echo "  Inception read-all: $((T1 - T0))ms"

echo ""
echo "╔═══════════════════════════════════════════════════╗"
echo "║  Benchmark Complete                              ║"
echo "╚═══════════════════════════════════════════════════╝"
echo ""
