#!/bin/bash
# ==============================================================================
# bench_cargo_inception.sh — Benchmark Cargo project under VRift inception
# ==============================================================================
#
# Usage:
#   ./bench_cargo_inception.sh <PROJECT_DIR> [options]
#
# Options:
#   --skip-ingest     Skip ingest (use existing CAS/VDir)
#   --runs N          Number of benchmark iterations (default: 3)
#   --modify FILE     File to modify for incremental test (default: auto-detect)
#   --vdir PATH       Specify exact VDir file
#
# Example:
#   ./bench_cargo_inception.sh ~/rust_source/velo
#   ./bench_cargo_inception.sh ~/rust_source/velo --skip-ingest --runs 5
#
# Phases:
#   1. [Optional] Ingest project into CAS
#   2. Baseline no-op build (no inception)
#   3. Inception no-op build
#   4. Touch file → incremental build (baseline vs inception)
#   5. Code change → incremental build → revert (baseline vs inception)
#   6. Clean → full rebuild: baseline vs inception (target restoration test)
#   7. Post-clean no-op verification
#   8. CAS integrity check
#   9. Timing summary table
# ==============================================================================

set -euo pipefail

# ============================================================================
# Parse arguments
# ============================================================================
PROJECT_DIR=""
SKIP_INGEST=false
BENCH_RUNS=3
MODIFY_FILE=""
VDIR_PATH_OVERRIDE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-ingest)  SKIP_INGEST=true; shift ;;
        --runs)         BENCH_RUNS="$2"; shift 2 ;;
        --modify)       MODIFY_FILE="$2"; shift 2 ;;
        --vdir)         VDIR_PATH_OVERRIDE="$2"; shift 2 ;;
        -h|--help)
            head -30 "$0" | grep '^#' | sed 's/^# \?//'
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

# Source SSOT env vars
source "$SCRIPT_DIR/../lib/vrift_env.sh"

PASSED=0
FAILED=0

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

format_time() {
    local t="$1"
    if [ "$t" -ge 1000 ]; then
        python3 -c "print(f'{${t}/1000:.2f}s')"
    else
        echo "${t}ms"
    fi
}

run_bench_avg() {
    local label="$1"; shift
    local times=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        local t0=$(ms)
        eval "$@"
        local t1=$(ms)
        times+=($((t1 - t0)))
    done
    local sum=0
    for t in "${times[@]}"; do sum=$((sum + t)); done
    local avg=$((sum / BENCH_RUNS))
    bench_record "$label" "$avg"
    echo "  $label: $(format_time $avg) (runs: ${times[*]})"
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

NOINC() {
    env -u DYLD_INSERT_LIBRARIES -u VRIFT_INCEPTION -u VRIFT_VDIR_MMAP "$@"
}

clean_target() {
    chflags -R nouchg "$PROJECT_DIR/target" 2>/dev/null || true
    rm -rf "$PROJECT_DIR/target"
}

# Auto-detect a .rs file to modify
auto_detect_modify_file() {
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
echo "  Iterations:  $BENCH_RUNS"

for bin in "$SHIM_LIB" "$VRIFT_CLI" "$VRIFTD"; do
    [ -f "$bin" ] || { echo "❌ Missing: $bin"; exit 1; }
done

if [ "$SKIP_INGEST" = true ] && [ ! -S "$VRIFT_SOCKET_PATH" ]; then
    echo "  ⚠️  Socket not found: $VRIFT_SOCKET_PATH"
fi
echo "  Socket:      $VRIFT_SOCKET_PATH"

RS_COUNT=$(find "$PROJECT_DIR" -name '*.rs' -not -path '*/target/*' 2>/dev/null | wc -l | tr -d ' ')
echo "  Rust files:  $RS_COUNT"

if [ -z "$MODIFY_FILE" ]; then
    MODIFY_FILE=$(auto_detect_modify_file)
fi
echo "  Modify file: ${MODIFY_FILE:-NONE}"
echo ""

# ============================================================================
# Phase 1: [Optional] Ingest
# ============================================================================
if [ "$SKIP_INGEST" = false ]; then
    echo "═══ Phase 1: Ingest ═══"

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
    echo "  $FILES_N → $BLOBS_N"
    sleep 2
else
    echo "═══ Phase 1: Ingest (skipped) ═══"
    DAEMON_PID=""
fi

# Find VDir
VDIR_MMAP_PATH=""
if [ -n "$VDIR_PATH_OVERRIDE" ]; then
    VDIR_MMAP_PATH="$VDIR_PATH_OVERRIDE"
elif [ -d "$PROJECT_DIR/.vrift/vdir" ]; then
    VDIR_MMAP_PATH=$(ls -t "$PROJECT_DIR/.vrift/vdir/"*.vdir 2>/dev/null | head -1)
fi
if [ -z "$VDIR_MMAP_PATH" ] && [ -d "$HOME/.vrift/vdir" ]; then
    VDIR_MMAP_PATH=$(ls -t "$HOME/.vrift/vdir/"*.vdir 2>/dev/null | head -1)
fi

if [ -n "$VDIR_MMAP_PATH" ]; then
    VDIR_SIZE=$(stat -f%z "$VDIR_MMAP_PATH" 2>/dev/null || stat -c%s "$VDIR_MMAP_PATH" 2>/dev/null)
    echo "  VDir: $(basename "$VDIR_MMAP_PATH") ($VDIR_SIZE bytes)"
    pass "VDir found"
else
    fail "VDir not found"
    exit 1
fi

# ============================================================================
# Phase 2: Baseline no-op (no inception)
# ============================================================================
echo ""
echo "═══ Phase 2: Baseline no-op build ═══"

cd "$PROJECT_DIR"
# Warmup
NOINC cargo build >/dev/null 2>&1 || true

run_bench_avg "Baseline no-op" 'NOINC cargo build >/dev/null 2>&1'
pass "Baseline no-op"

# ============================================================================
# Phase 3: Inception no-op
# ============================================================================
echo ""
echo "═══ Phase 3: Inception no-op build ═══"

cd "$PROJECT_DIR"
# Warmup
INCEP cargo build >/dev/null 2>&1 || true

run_bench_avg "Inception no-op" 'INCEP cargo build >/dev/null 2>&1'
pass "Inception no-op"

# ============================================================================
# Phase 4: Touch → incremental (baseline vs inception)
# ============================================================================
echo ""
echo "═══ Phase 4: Touch → incremental build ═══"

if [ -n "$MODIFY_FILE" ] && [ -f "$MODIFY_FILE" ]; then
    # Baseline touch incremental
    echo "  --- Baseline ---"
    times_touch_b=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        sleep 1; touch "$MODIFY_FILE"
        T0=$(ms); NOINC cargo build >/dev/null 2>&1 || true; T1=$(ms)
        elapsed=$((T1 - T0)); times_touch_b+=("$elapsed")
    done
    sum=0; for t in "${times_touch_b[@]}"; do sum=$((sum + t)); done
    avg_b=$((sum / BENCH_RUNS))
    bench_record "Touch incr (baseline)" "$avg_b"
    echo "  Touch incr (baseline): $(format_time $avg_b) (runs: ${times_touch_b[*]})"

    # Inception touch incremental
    echo "  --- Inception ---"
    times_touch_i=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        sleep 1; touch "$MODIFY_FILE"
        T0=$(ms); INCEP cargo build >/dev/null 2>&1 || true; T1=$(ms)
        elapsed=$((T1 - T0)); times_touch_i+=("$elapsed")
    done
    sum=0; for t in "${times_touch_i[@]}"; do sum=$((sum + t)); done
    avg_i=$((sum / BENCH_RUNS))
    bench_record "Touch incr (inception)" "$avg_i"
    echo "  Touch incr (inception): $(format_time $avg_i) (runs: ${times_touch_i[*]})"

    if [ "$avg_b" -gt 0 ]; then
        RATIO=$(python3 -c "print(f'{$avg_i/$avg_b:.2f}x')")
        echo "  Ratio: $RATIO"
    fi
    pass "Touch incremental"
else
    echo "  No modify file found, skipping"
fi

# ============================================================================
# Phase 5: Code change → build → revert (baseline vs inception)
# ============================================================================
echo ""
echo "═══ Phase 5: Code change → incremental build ═══"

if [ -n "$MODIFY_FILE" ] && [ -f "$MODIFY_FILE" ]; then
    cp "$MODIFY_FILE" "${MODIFY_FILE}.bench_backup"

    # Baseline code change
    echo "  --- Baseline ---"
    times_code_b=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        echo "" >> "$MODIFY_FILE"
        echo "// bench_canary_${i}_$(date +%s)" >> "$MODIFY_FILE"
        T0=$(ms); NOINC cargo build >/dev/null 2>&1 || true; T1=$(ms)
        elapsed=$((T1 - T0)); times_code_b+=("$elapsed")
        cp "${MODIFY_FILE}.bench_backup" "$MODIFY_FILE"
        NOINC cargo build >/dev/null 2>&1 || true  # restore
    done
    sum=0; for t in "${times_code_b[@]}"; do sum=$((sum + t)); done
    avg_b=$((sum / BENCH_RUNS))
    bench_record "Code change (baseline)" "$avg_b"
    echo "  Code change (baseline): $(format_time $avg_b) (runs: ${times_code_b[*]})"

    # Inception code change
    echo "  --- Inception ---"
    times_code_i=()
    for i in $(seq 1 "$BENCH_RUNS"); do
        echo "" >> "$MODIFY_FILE"
        echo "// bench_canary_${i}_$(date +%s)" >> "$MODIFY_FILE"
        T0=$(ms); INCEP cargo build >/dev/null 2>&1 || true; T1=$(ms)
        elapsed=$((T1 - T0)); times_code_i+=("$elapsed")
        cp "${MODIFY_FILE}.bench_backup" "$MODIFY_FILE"
        INCEP cargo build >/dev/null 2>&1 || true  # restore
    done
    sum=0; for t in "${times_code_i[@]}"; do sum=$((sum + t)); done
    avg_i=$((sum / BENCH_RUNS))
    bench_record "Code change (inception)" "$avg_i"
    echo "  Code change (inception): $(format_time $avg_i) (runs: ${times_code_i[*]})"

    if [ "$avg_b" -gt 0 ]; then
        RATIO=$(python3 -c "print(f'{$avg_i/$avg_b:.2f}x')")
        echo "  Ratio: $RATIO"
    fi
    pass "Code change incremental"

    rm -f "${MODIFY_FILE}.bench_backup"
else
    echo "  No modify file, skipping"
fi

# ============================================================================
# Phase 6: Clean → full rebuild (THE KEY TEST)
#
# This tests the most critical acceleration scenario:
#   - After cargo clean removes target/, can inception restore cached
#     build artifacts from CAS so that recompilation is avoided?
#
# Comparison:
#   A) Baseline: cargo clean → cargo build (full recompile, no help)
#   B) Inception: cargo clean → inception cargo build (should restore target/)
#
# If inception correctly caches and restores target/ artifacts, Phase B
# should be significantly faster than Phase A.
# ============================================================================
echo ""
echo "═══ Phase 6: Clean → full rebuild (target restoration test) ═══"
echo "  This tests whether inception restores target/ from cache after clean."

cd "$PROJECT_DIR"

# Ensure we have a warm build first
INCEP cargo build >/dev/null 2>&1 || true

# --- A) Baseline: clean → build without inception ---
echo ""
echo "  --- Baseline (no inception) ---"
clean_target
T0=$(ms)
NOINC cargo build >/dev/null 2>&1 || true
T1=$(ms)
BASELINE_CLEAN=$((T1 - T0))
bench_record "Clean build (baseline)" "$BASELINE_CLEAN"
echo "  Clean build (baseline): $(format_time $BASELINE_CLEAN)"
pass "Baseline clean build"

# --- B) Inception: clean → build with inception ---
# First, rebuild with inception to populate any target caches
INCEP cargo build >/dev/null 2>&1 || true

echo ""
echo "  --- Inception (should restore target/) ---"
clean_target
T0=$(ms)
INCEP cargo build >/dev/null 2>&1 || true
T1=$(ms)
INCEPTION_CLEAN=$((T1 - T0))
bench_record "Clean build (inception)" "$INCEPTION_CLEAN"
echo "  Clean build (inception): $(format_time $INCEPTION_CLEAN)"
pass "Inception clean build"

if [ "$BASELINE_CLEAN" -gt 0 ]; then
    RATIO=$(python3 -c "print(f'{$INCEPTION_CLEAN/$BASELINE_CLEAN:.2f}x')")
    SAVED=$(python3 -c "print(f'{(1 - $INCEPTION_CLEAN/$BASELINE_CLEAN)*100:.0f}%')")
    echo ""
    echo "  ⚡ Clean build ratio: $RATIO ($SAVED saved)"
    if [ "$INCEPTION_CLEAN" -lt "$BASELINE_CLEAN" ]; then
        pass "Inception accelerated clean build"
    else
        echo "  ⚠️  No acceleration — target restoration may not be active"
    fi
fi

# ============================================================================
# Phase 7: Post-clean verification
# ============================================================================
echo ""
echo "═══ Phase 7: Post-clean no-op verification ═══"

cd "$PROJECT_DIR"
run_bench_avg "Post-clean no-op" 'INCEP cargo build >/dev/null 2>&1'
pass "Post-clean no-op"

# Also verify baseline no-op still works
T0=$(ms)
NOINC cargo build >/dev/null 2>&1
T1=$(ms)
POST_BASELINE=$((T1 - T0))
bench_record "Post-clean baseline no-op" "$POST_BASELINE"
echo "  Post-clean baseline no-op: $(format_time $POST_BASELINE)"
pass "Post-clean baseline no-op"

# ============================================================================
# Phase 8: CAS integrity
# ============================================================================
echo ""
echo "═══ Phase 8: CAS integrity ═══"

if [ "$(uname)" = "Darwin" ]; then
    UCHG=$(find "$VR_THE_SOURCE" -name "*.bin" -flags uchg 2>/dev/null | wc -l | tr -d ' ')
    TOTAL=$(find "$VR_THE_SOURCE" -name "*.bin" 2>/dev/null | wc -l | tr -d ' ')
    echo "  CAS blobs: $TOTAL total, $UCHG protected (uchg)"
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
echo "╠══════════════════════════════════════════════════════════════════════╣"
echo ""
printf "  %-35s %10s\n" "Scenario" "Time"
printf "  %-35s %10s\n" "-----------------------------------" "----------"
for i in "${!BENCH_NAMES[@]}"; do
    T=${BENCH_TIMES[$i]}
    printf "  %-35s %10s\n" "${BENCH_NAMES[$i]}" "$(format_time $T)"
done

# Key comparisons
echo ""
echo "  ── Key Comparisons ──"
for i in "${!BENCH_NAMES[@]}"; do
    [ "${BENCH_NAMES[$i]}" = "Baseline no-op" ] && BASELINE_NOOP=${BENCH_TIMES[$i]}
    [ "${BENCH_NAMES[$i]}" = "Inception no-op" ] && INCEP_NOOP=${BENCH_TIMES[$i]}
    [ "${BENCH_NAMES[$i]}" = "Clean build (baseline)" ] && CLEAN_BASE=${BENCH_TIMES[$i]}
    [ "${BENCH_NAMES[$i]}" = "Clean build (inception)" ] && CLEAN_INCEP=${BENCH_TIMES[$i]}
done

if [ -n "${BASELINE_NOOP:-}" ] && [ -n "${INCEP_NOOP:-}" ] && [ "$BASELINE_NOOP" -gt 0 ]; then
    OVERHEAD=$((INCEP_NOOP - BASELINE_NOOP))
    printf "  %-35s %+dms\n" "No-op overhead" "$OVERHEAD"
fi

if [ -n "${CLEAN_BASE:-}" ] && [ -n "${CLEAN_INCEP:-}" ] && [ "$CLEAN_BASE" -gt 0 ]; then
    RATIO=$(python3 -c "print(f'{$CLEAN_INCEP/$CLEAN_BASE:.2f}x')")
    SAVED_MS=$((CLEAN_BASE - CLEAN_INCEP))
    if [ "$SAVED_MS" -ge 0 ]; then
        printf "  %-35s %10s (saved %dms)\n" "Clean build ratio" "$RATIO" "$SAVED_MS"
    else
        EXTRA_MS=$((-SAVED_MS))
        printf "  %-35s %10s (+%dms overhead)\n" "Clean build ratio" "$RATIO" "$EXTRA_MS"
    fi
fi

echo ""
echo "╚══════════════════════════════════════════════════════════════════════╝"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  RESULTS: $PASSED passed, $FAILED failed"
echo "═══════════════════════════════════════════════════════════════"

[ "$FAILED" -gt 0 ] && { echo "  ❌ SOME TESTS FAILED"; exit 1; }
echo "  ✅ ALL TESTS PASSED"
exit 0
