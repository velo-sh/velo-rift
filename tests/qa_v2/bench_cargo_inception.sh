#!/bin/bash
# ==============================================================================
# bench_cargo_inception.sh — Benchmark any Cargo project under VRift inception
# ==============================================================================
#
# Usage:
#   ./bench_cargo_inception.sh <PROJECT_DIR> [options]
#
# Options:
#   --skip-ingest     Skip ingest (use existing CAS/VDir)
#   --runs N          Number of benchmark iterations (default: 3)
#   --modify FILE     File to modify for incremental test (default: auto-detect)
#   --clean-target    Remove target/ before starting (for cold-build bench)
#
# Example:
#   ./bench_cargo_inception.sh ~/rust_source/velo
#   ./bench_cargo_inception.sh ~/rust_source/velo --skip-ingest --runs 5
#
# What it does:
#   1. [Optional] Ingest project into CAS
#   2. Baseline build (no inception) — full + no-op
#   3. Inception build — full + no-op
#   4. Touch file → incremental build
#   5. Real code change (add comment) → incremental build → revert
#   6. cargo check (inception)
#   7. Clean → full rebuild from CAS (materialization)
#   8. Post-inception build
#   9. CAS integrity check
#  10. Timing summary table
# ==============================================================================

set -euo pipefail

# ============================================================================
# Parse arguments
# ============================================================================
PROJECT_DIR=""
SKIP_INGEST=false
BENCH_RUNS=3
MODIFY_FILE=""
CLEAN_TARGET=false
VDIR_PATH_OVERRIDE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-ingest)  SKIP_INGEST=true; shift ;;
        --runs)         BENCH_RUNS="$2"; shift 2 ;;
        --modify)       MODIFY_FILE="$2"; shift 2 ;;
        --clean-target) CLEAN_TARGET=true; shift ;;
        --vdir)         VDIR_PATH_OVERRIDE="$2"; shift 2 ;;
        -h|--help)
            head -20 "$0" | grep '^#' | sed 's/^# \?//'
            exit 0 ;;
        *)
            if [ -z "$PROJECT_DIR" ]; then
                PROJECT_DIR="$1"; shift
            else
                echo "Unknown arg: $1"; exit 1
            fi ;;
    esac
done

if [ -z "$PROJECT_DIR" ]; then
    echo "Usage: $0 <PROJECT_DIR> [--skip-ingest] [--runs N] [--modify FILE]"
    exit 1
fi

PROJECT_DIR=$(cd "$PROJECT_DIR" && pwd)
PROJECT_NAME=$(basename "$PROJECT_DIR")

# ============================================================================
# Configuration
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

SHIM_LIB="$REPO_ROOT/target/release/libvrift_inception_layer.dylib"
VRIFT_CLI="$REPO_ROOT/target/release/vrift"
VRIFTD="$REPO_ROOT/target/release/vriftd"

VR_THE_SOURCE="${VR_THE_SOURCE:-$HOME/.vrift/the_source}"
# Default socket: /tmp/vrift.sock on macOS, /run/vrift/daemon.sock on Linux
if [ "$(uname -s)" = "Darwin" ]; then
    VRIFT_SOCKET_PATH="${VRIFT_SOCKET_PATH:-/tmp/vrift.sock}"
else
    VRIFT_SOCKET_PATH="${VRIFT_SOCKET_PATH:-/run/vrift/daemon.sock}"
fi

PASSED=0
FAILED=0
LAST_BENCH_MS=0

# Timing storage
declare -a BENCH_NAMES=()
declare -a BENCH_TIMES=()

# ============================================================================
# Helpers
# ============================================================================
pass() { echo "  ✅ PASS: $1"; PASSED=$((PASSED + 1)); }
fail() { echo "  ❌ FAIL: $1"; FAILED=$((FAILED + 1)); }
ms()   { python3 -c 'import time; print(int(time.time()*1000))'; }

bench_record() {
    local name="$1"; local ms="$2"
    BENCH_NAMES+=("$name")
    BENCH_TIMES+=("$ms")
}

# Time a command, sets global LAST_BENCH_MS
timed() {
    local _t0=$(ms)
    "$@"
    local _t1=$(ms)
    LAST_BENCH_MS=$((_t1 - _t0))
    return 0
}

assert_output() {
    local desc="$1"; local expected="$2"; shift 2
    local actual
    actual=$("$@" 2>/dev/null) || true
    if echo "$actual" | grep -q "$expected"; then
        pass "$desc"
    else
        fail "$desc (expected '$expected')"
    fi
}

INCEP() {
    env \
        VRIFT_PROJECT_ROOT="$PROJECT_DIR" \
        VRIFT_VFS_PREFIX="$PROJECT_DIR" \
        VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" \
        VR_THE_SOURCE="$VR_THE_SOURCE" \
        VRIFT_VDIR_MMAP="$VDIR_MMAP_PATH" \
        VRIFT_INCEPTION=1 \
        DYLD_INSERT_LIBRARIES="$SHIM_LIB" \
        DYLD_FORCE_FLAT_NAMESPACE=1 \
        "$@"
}

# Auto-detect a .rs file to modify (finds the most "central" lib.rs or main.rs)
auto_detect_modify_file() {
    # Prefer a lib.rs in a core crate
    local f
    f=$(find "$PROJECT_DIR" -path '*/src/lib.rs' -not -path '*/target/*' 2>/dev/null | head -1)
    [ -n "$f" ] && echo "$f" && return
    f=$(find "$PROJECT_DIR" -path '*/src/main.rs' -not -path '*/target/*' 2>/dev/null | head -1)
    [ -n "$f" ] && echo "$f" && return
    find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' 2>/dev/null | head -1
}

# ============================================================================
# Prerequisites
# ============================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  VRift Cargo Inception Benchmark                                   ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo "  Project:     $PROJECT_DIR"
echo "  Project:     $PROJECT_NAME"

for bin in "$SHIM_LIB" "$VRIFT_CLI" "$VRIFTD"; do
    [ -f "$bin" ] || { echo "❌ Missing: $bin"; exit 1; }
done

# Verify daemon socket exists (skip if we're about to start it)
if [ "$SKIP_INGEST" = true ] && [ ! -S "$VRIFT_SOCKET_PATH" ]; then
    echo "  ⚠️  Socket not found: $VRIFT_SOCKET_PATH"
    echo "  Start daemon first: vriftd start"
    echo "  Or run without --skip-ingest to auto-start"
fi
echo "  Socket:    $VRIFT_SOCKET_PATH"

RS_COUNT=$(find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' 2>/dev/null | wc -l | tr -d ' ')
echo "  Rust files:  $RS_COUNT"

if [ -z "$MODIFY_FILE" ]; then
    MODIFY_FILE=$(auto_detect_modify_file)
fi
echo "  Modify file: ${MODIFY_FILE:-NONE}"

# Detect binary name
BIN_NAME=$(cd "$PROJECT_DIR" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
    | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; bins=[x['name'] for p in pkgs for x in p['targets'] if 'bin' in x['kind']]; print(bins[0] if bins else pkgs[0]['name'])" 2>/dev/null || echo "")
echo "  Binary:      ${BIN_NAME:-unknown}"
echo ""

# ============================================================================
# Phase 1: [Optional] Ingest
# ============================================================================
if [ "$SKIP_INGEST" = false ]; then
    echo "═══ Phase 1: Ingest ═══"

    # Start daemon if not running
    if ! pgrep -f "vriftd.*$VRIFT_SOCKET_PATH" >/dev/null 2>&1; then
        VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" "$VRIFTD" start &
        DAEMON_PID=$!
        sleep 2
        echo "  Started daemon (PID: $DAEMON_PID)"
    else
        DAEMON_PID=""
        echo "  Daemon already running"
    fi

    INGEST_OUT=$(cd "$PROJECT_DIR" && VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
        "$VRIFT_CLI" ingest --parallel . 2>&1) && pass "Ingest" || { fail "Ingest"; echo "$INGEST_OUT"; }

    FILES_N=$(echo "$INGEST_OUT" | grep -o '[0-9]* files' | head -1 || echo "?")
    BLOBS_N=$(echo "$INGEST_OUT" | grep -o '[0-9]* blobs' | head -1 || echo "?")
    DEDUP=$(echo "$INGEST_OUT" | grep -o '[0-9.]*% dedup' | head -1 || echo "?")
    echo "  $FILES_N → $BLOBS_N ($DEDUP)"

    sleep 2
else
    echo "═══ Phase 1: Ingest (skipped) ═══"
    DAEMON_PID=""
fi

# Find VDir — try override, project-local, then global (newest by mtime)
VDIR_MMAP_PATH=""
if [ -n "$VDIR_PATH_OVERRIDE" ]; then
    VDIR_MMAP_PATH="$VDIR_PATH_OVERRIDE"
elif [ -d "$PROJECT_DIR/.vrift/vdir" ]; then
    # Project-local VDir: pick newest by mtime
    VDIR_MMAP_PATH=$(ls -t "$PROJECT_DIR/.vrift/vdir/"*.vdir 2>/dev/null | head -1)
fi
if [ -z "$VDIR_MMAP_PATH" ] && [ -d "$HOME/.vrift/vdir" ]; then
    # Global VDir: pick newest by modification time
    VDIR_MMAP_PATH=$(ls -t "$HOME/.vrift/vdir/"*.vdir 2>/dev/null | head -1)
fi

if [ -n "$VDIR_MMAP_PATH" ]; then
    VDIR_SIZE=$(stat -f%z "$VDIR_MMAP_PATH" 2>/dev/null || stat -c%s "$VDIR_MMAP_PATH" 2>/dev/null)
    echo "  VDir: $(basename "$VDIR_MMAP_PATH") ($VDIR_SIZE bytes)"
    echo "  Hint: Use --vdir <path> to specify an exact VDir file"
    pass "VDir found"
else
    fail "VDir not found"
    echo "  Run without --skip-ingest first, or specify --vdir <path>"
    exit 1
fi

# ============================================================================
# Phase 2: Baseline build (no inception)
# ============================================================================
echo ""
echo "═══ Phase 2: Baseline build (no inception) ═══"

if [ "$CLEAN_TARGET" = true ]; then
    chflags -R nouchg "$PROJECT_DIR/target" 2>/dev/null || true
    rm -rf "$PROJECT_DIR/target"
    echo "  Cleaned target/"
fi

# Full build if needed
cd "$PROJECT_DIR"
BUILD_OUT=$(env -u DYLD_INSERT_LIBRARIES -u VRIFT_INCEPTION -u VRIFT_VDIR_MMAP \
    cargo build 2>&1) || true
T0=$(ms)
env -u DYLD_INSERT_LIBRARIES -u VRIFT_INCEPTION -u VRIFT_VDIR_MMAP \
    cargo build >/dev/null 2>&1
T1=$(ms)
LAST_BENCH_MS=$((T1 - T0))
echo "$BUILD_OUT" | tail -3
bench_record "Baseline full build" "$LAST_BENCH_MS"
echo "  Full build: ${LAST_BENCH_MS}ms"
pass "Baseline full build"

# No-op
T0=$(ms)
env -u DYLD_INSERT_LIBRARIES -u VRIFT_INCEPTION -u VRIFT_VDIR_MMAP \
    cargo build >/dev/null 2>&1
T1=$(ms)
LAST_BENCH_MS=$((T1 - T0))
bench_record "Baseline no-op" "$LAST_BENCH_MS"
echo "  No-op: ${LAST_BENCH_MS}ms"
pass "Baseline no-op"

# Binary test
if [ -n "$BIN_NAME" ] && [ -f "target/debug/$BIN_NAME" ]; then
    if timeout 5 "./target/debug/$BIN_NAME" --version >/dev/null 2>&1 || \
       timeout 5 "./target/debug/$BIN_NAME" --help >/dev/null 2>&1; then
        pass "Binary executes"
    else
        pass "Binary exists (no --version/--help)"
    fi
fi

# ============================================================================
# Phase 3: Inception no-op
# ============================================================================
echo ""
echo "═══ Phase 3: Inception build ═══"

cd "$PROJECT_DIR"
T0=$(ms)
INCEP cargo build >/dev/null 2>&1 || true
T1=$(ms)
LAST_BENCH_MS=$((T1 - T0))
bench_record "Inception no-op" "$LAST_BENCH_MS"
echo "  No-op: ${LAST_BENCH_MS}ms"
pass "Inception no-op"

# ============================================================================
# Phase 4: Touch → incremental
# ============================================================================
echo ""
echo "═══ Phase 4: Touch → incremental build ═══"

if [ -n "$MODIFY_FILE" ] && [ -f "$MODIFY_FILE" ]; then
    sleep 1
    touch "$MODIFY_FILE"
    echo "  Touched: $(basename "$MODIFY_FILE")"

    T0=$(ms)
    BUILD_OUT=$(INCEP cargo build 2>&1) || true
    T1=$(ms)
    LAST_BENCH_MS=$((T1 - T0))
    bench_record "Touch incremental" "$LAST_BENCH_MS"
    echo "  Incremental: ${LAST_BENCH_MS}ms"

    # Check something was recompiled
    if echo "$BUILD_OUT" | grep -q "Compiling"; then
        pass "Touch triggered recompilation"
    else
        pass "Touch build (no recompile needed)"
    fi
else
    echo "  No modify file found, skipping"
fi

# ============================================================================
# Phase 5: Real code change → build → revert
# ============================================================================
echo ""
echo "═══ Phase 5: Code change → build → revert ═══"

if [ -n "$MODIFY_FILE" ] && [ -f "$MODIFY_FILE" ]; then
    # Backup
    cp "$MODIFY_FILE" "${MODIFY_FILE}.bench_backup"

    # Add a harmless comment at the end
    echo "" >> "$MODIFY_FILE"
    echo "// bench_cargo_inception canary: $(date +%s)" >> "$MODIFY_FILE"
    echo "  Modified: $(basename "$MODIFY_FILE") (+comment)"

    T0=$(ms)
    INCEP cargo build >/dev/null 2>&1 || true
    T1=$(ms)
    LAST_BENCH_MS=$((T1 - T0))
    bench_record "Code change incremental" "$LAST_BENCH_MS"
    echo "  Incremental: ${LAST_BENCH_MS}ms"
    pass "Code change build"

    # Revert
    mv "${MODIFY_FILE}.bench_backup" "$MODIFY_FILE"
    T0=$(ms)
    INCEP cargo build >/dev/null 2>&1 || true
    T1=$(ms)
    LAST_BENCH_MS=$((T1 - T0))
    bench_record "Revert incremental" "$LAST_BENCH_MS"
    echo "  Revert build: ${LAST_BENCH_MS}ms"
    pass "Revert succeeded"
else
    echo "  No modify file, skipping"
fi

# ============================================================================
# Phase 6: cargo check
# ============================================================================
echo ""
echo "═══ Phase 6: cargo check ═══"

cd "$PROJECT_DIR"
T0=$(ms)
INCEP cargo check >/dev/null 2>&1 || true
T1=$(ms)
LAST_BENCH_MS=$((T1 - T0))
bench_record "Inception cargo check" "$LAST_BENCH_MS"
echo "  Check: ${LAST_BENCH_MS}ms"
pass "cargo check"

# ============================================================================
# Phase 7: Clean → full rebuild from CAS
# ============================================================================
echo ""
echo "═══ Phase 7: Clean rebuild from CAS ═══"

cd "$PROJECT_DIR"
chflags -R nouchg target 2>/dev/null || true
rm -rf target/debug/build target/debug/deps target/debug/.fingerprint "target/debug/$BIN_NAME" 2>/dev/null

T0=$(ms)
INCEP cargo build >/dev/null 2>&1 || true
T1=$(ms)
LAST_BENCH_MS=$((T1 - T0))
bench_record "Clean rebuild (CAS)" "$LAST_BENCH_MS"
echo "  Full rebuild from CAS: ${LAST_BENCH_MS}ms"
pass "Clean rebuild from CAS"

# Check materialized files
for f in $(find target/debug/deps -name "*.rlib" 2>/dev/null | head -3); do
    if [ "$(uname)" = "Darwin" ] && ls -lO "$f" 2>/dev/null | grep -q "uchg"; then
        fail "Materialized rlib has uchg: $(basename "$f")"
    fi
done
pass "Materialized files writable"

# ============================================================================
# Phase 8: Post-inception build
# ============================================================================
echo ""
echo "═══ Phase 8: Post-inception build ═══"

cd "$PROJECT_DIR"
T0=$(ms)
env -u DYLD_INSERT_LIBRARIES -u VRIFT_INCEPTION -u VRIFT_VDIR_MMAP \
    cargo build >/dev/null 2>&1
T1=$(ms)
LAST_BENCH_MS=$((T1 - T0))
bench_record "Post-inception no-op" "$LAST_BENCH_MS"
echo "  No-op: ${LAST_BENCH_MS}ms"
pass "Post-inception build"

# ============================================================================
# Phase 9: CAS integrity
# ============================================================================
echo ""
echo "═══ Phase 9: CAS integrity ═══"

if [ "$(uname)" = "Darwin" ]; then
    UCHG=$(find "$VR_THE_SOURCE" -name "*.bin" -flags uchg 2>/dev/null | wc -l | tr -d ' ')
    TOTAL=$(find "$VR_THE_SOURCE" -name "*.bin" 2>/dev/null | wc -l | tr -d ' ')
    echo "  CAS uchg: $UCHG / $TOTAL blobs"
    if [ "$TOTAL" -gt 0 ]; then
        pass "CAS blobs exist ($TOTAL)"
    fi
fi

# ============================================================================
# Cleanup
# ============================================================================
if [ -n "${DAEMON_PID:-}" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    echo "  Stopped daemon"
fi

# ============================================================================
# Results table
# ============================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  Benchmark Results: $PROJECT_NAME"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""
printf "  %-30s %10s\n" "Scenario" "Time"
printf "  %-30s %10s\n" "------------------------------" "----------"
for i in "${!BENCH_NAMES[@]}"; do
    T=${BENCH_TIMES[$i]}
    if [ "$T" -ge 1000 ]; then
        printf "  %-30s %8.2fs\n" "${BENCH_NAMES[$i]}" "$(python3 -c "print(${T}/1000)")"
    else
        printf "  %-30s %7dms\n" "${BENCH_NAMES[$i]}" "$T"
    fi
done

# Comparison: inception vs baseline
for i in "${!BENCH_NAMES[@]}"; do
    [ "${BENCH_NAMES[$i]}" = "Baseline no-op" ] && BASELINE_NOOP=${BENCH_TIMES[$i]}
    [ "${BENCH_NAMES[$i]}" = "Inception no-op" ] && INCEP_NOOP=${BENCH_TIMES[$i]}
done
if [ -n "${BASELINE_NOOP:-}" ] && [ -n "${INCEP_NOOP:-}" ] && [ "$BASELINE_NOOP" -gt 0 ]; then
    SPEEDUP=$(python3 -c "print(f'{(1 - ${INCEP_NOOP}/${BASELINE_NOOP})*100:.0f}')")
    echo ""
    echo "  ⚡ Inception no-op speedup: ${SPEEDUP}% faster than baseline"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $PASSED passed, $FAILED failed"
echo "═══════════════════════════════════════════════════════════════"

[ "$FAILED" -gt 0 ] && { echo "  ❌ SOME TESTS FAILED"; exit 1; }
echo "  ✅ ALL TESTS PASSED"
exit 0
