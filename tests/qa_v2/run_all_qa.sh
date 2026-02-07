#!/bin/bash
# ==============================================================================
# Velo Rift QA Suite Runner
# ==============================================================================
# Runs all test_*.sh scripts in the qa_v2 directory with per-test timeout,
# daemon cleanup between tests, and aggregate pass/fail summary.
#
# Usage:
#   ./run_all_qa.sh                    # Run all tests sequentially
#   ./run_all_qa.sh --parallel         # Run tests in parallel (4 workers)
#   ./run_all_qa.sh --parallel=8       # Run with 8 parallel workers
#   ./run_all_qa.sh --filter=boot      # Only run tests matching "boot"
#   ./run_all_qa.sh --timeout=180      # Set per-test timeout (seconds)
#   ./run_all_qa.sh --stress           # Include stress/repro_* tests
#   ./run_all_qa.sh --skip-build       # Skip cargo build step
#
# Environment:
#   VRIFT_QA_TIMEOUT=120        Per-test timeout in seconds (default: 120)
#   VRIFT_QA_PARALLEL=0         Number of parallel workers (0=sequential)
#   VRIFT_QA_FILTER=""          Filter test names (substring match)
#   VRIFT_QA_STRESS=0           Include stress tests (repro_*.sh)
#   VRIFT_QA_SKIP_BUILD=0       Skip cargo build
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ============================================================================
# CLI Argument Parsing
# ============================================================================
TIMEOUT="${VRIFT_QA_TIMEOUT:-120}"
PARALLEL="${VRIFT_QA_PARALLEL:-0}"
FILTER="${VRIFT_QA_FILTER:-}"
STRESS="${VRIFT_QA_STRESS:-0}"
SKIP_BUILD="${VRIFT_QA_SKIP_BUILD:-0}"

for arg in "$@"; do
    case "$arg" in
        --parallel)    PARALLEL=4 ;;
        --parallel=*)  PARALLEL="${arg#*=}" ;;
        --filter=*)    FILTER="${arg#*=}" ;;
        --timeout=*)   TIMEOUT="${arg#*=}" ;;
        --stress)      STRESS=1 ;;
        --skip-build)  SKIP_BUILD=1 ;;
        --help|-h)
            head -20 "$0" | tail -15
            exit 0
            ;;
        *)
            echo "Unknown argument: $arg"
            exit 1
            ;;
    esac
done

# ============================================================================
# Banner
# ============================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║              Velo Rift QA Suite Runner                             ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""
echo "  Project Root:   $PROJECT_ROOT"
echo "  Timeout:        ${TIMEOUT}s per test"
echo "  Parallel:       ${PARALLEL:-sequential}"
echo "  Filter:         ${FILTER:-<all>}"
echo "  Stress tests:   $([ "$STRESS" = "1" ] && echo "included" || echo "excluded")"
echo ""

# ============================================================================
# Build (unless skipped)
# ============================================================================
if [ "$SKIP_BUILD" != "1" ]; then
    echo "🔨 Building workspace..."
    (cd "$PROJECT_ROOT" && cargo build --release -p vrift-inception-layer -p vrift-cli -p vrift-daemon 2>&1) || {
        echo "❌ Build failed. Fix compilation errors before running QA."
        exit 1
    }
    echo "   ✓ Build complete"
    echo ""
fi

# ============================================================================
# Discover Tests
# ============================================================================
TESTS=()
for test_script in "$SCRIPT_DIR"/test_*.sh; do
    [ -f "$test_script" ] || continue
    name="$(basename "$test_script")"

    # Apply filter
    if [ -n "$FILTER" ] && [[ "$name" != *"$FILTER"* ]]; then
        continue
    fi

    TESTS+=("$test_script")
done

# Include stress tests if requested
if [ "$STRESS" = "1" ]; then
    for stress_script in "$SCRIPT_DIR"/repro_*.sh; do
        [ -f "$stress_script" ] || continue
        name="$(basename "$stress_script")"
        if [ -n "$FILTER" ] && [[ "$name" != *"$FILTER"* ]]; then
            continue
        fi
        TESTS+=("$stress_script")
    done
fi

# Also include check_stack_frame.sh (not a test_* but critical)
if [ -f "$SCRIPT_DIR/check_stack_frame.sh" ]; then
    if [ -z "$FILTER" ] || [[ "check_stack_frame" == *"$FILTER"* ]]; then
        TESTS+=("$SCRIPT_DIR/check_stack_frame.sh")
    fi
fi

TOTAL=${#TESTS[@]}
echo "📋 Found $TOTAL test(s) to run"
echo ""

if [ "$TOTAL" -eq 0 ]; then
    echo "⚠️  No tests matched filter: '$FILTER'"
    exit 0
fi

# ============================================================================
# Daemon Cleanup Helper
# ============================================================================
cleanup_stray_daemons() {
    pkill -f "vriftd.*vrift_test" 2>/dev/null || true
    # Brief pause to let processes exit
    sleep 0.2
}

# ============================================================================
# Run Single Test
# ============================================================================
run_test() {
    local test_script="$1"
    local name
    name="$(basename "$test_script")"
    local log_file="/tmp/vrift_qa_${name%.sh}.log"

    # Clean up stray daemons from previous test
    cleanup_stray_daemons

    local start_time
    start_time=$(date +%s)

    # Run with timeout
    local exit_code=0
    if command -v gtimeout &>/dev/null; then
        gtimeout "$TIMEOUT" bash "$test_script" > "$log_file" 2>&1 || exit_code=$?
    elif command -v timeout &>/dev/null; then
        timeout "$TIMEOUT" bash "$test_script" > "$log_file" 2>&1 || exit_code=$?
    else
        # macOS fallback: use perl-based timeout
        (
            bash "$test_script" > "$log_file" 2>&1 &
            local pid=$!
            (sleep "$TIMEOUT" && kill -TERM "$pid" 2>/dev/null && echo "TIMEOUT" >> "$log_file") &
            local watchdog=$!
            wait "$pid" 2>/dev/null
            local code=$?
            kill "$watchdog" 2>/dev/null || true
            exit $code
        ) || exit_code=$?
    fi

    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))

    # Clean up after test
    cleanup_stray_daemons

    if [ $exit_code -eq 0 ]; then
        echo "  ✅ PASS  ${name}  (${duration}s)"
        return 0
    elif [ $exit_code -eq 124 ] || [ $exit_code -eq 137 ]; then
        echo "  ⏱️  TIMEOUT  ${name}  (>${TIMEOUT}s)"
        echo "    └─ Log: $log_file"
        return 1
    else
        echo "  ❌ FAIL  ${name}  (${duration}s, exit=$exit_code)"
        # Show last 5 lines of output for quick triage
        echo "    └─ Log: $log_file"
        tail -5 "$log_file" 2>/dev/null | sed 's/^/    │  /'
        return 1
    fi
}

# ============================================================================
# Sequential Execution
# ============================================================================
run_sequential() {
    local passed=0
    local failed=0
    local failed_names=()

    for test_script in "${TESTS[@]}"; do
        if run_test "$test_script"; then
            passed=$((passed + 1))
        else
            failed=$((failed + 1))
            failed_names+=("$(basename "$test_script")")
        fi
    done

    print_summary $passed $failed "${failed_names[@]}"
}

# ============================================================================
# Parallel Execution
# ============================================================================
run_parallel() {
    local max_jobs="$PARALLEL"
    local passed=0
    local failed=0
    local failed_names=()
    local pids=()
    local scripts=()
    local results_dir
    results_dir="$(mktemp -d /tmp/vrift_qa_parallel_XXXXX)"

    echo "🚀 Running $TOTAL tests with $max_jobs parallel workers"
    echo ""

    for test_script in "${TESTS[@]}"; do
        local name
        name="$(basename "$test_script")"
        local result_file="$results_dir/$name"

        # Throttle: wait if we have max_jobs running
        while [ ${#pids[@]} -ge "$max_jobs" ]; do
            local new_pids=()
            for pid in "${pids[@]}"; do
                if kill -0 "$pid" 2>/dev/null; then
                    new_pids+=("$pid")
                fi
            done
            pids=("${new_pids[@]}")
            [ ${#pids[@]} -ge "$max_jobs" ] && sleep 0.5
        done

        # Launch test in background
        (
            if run_test "$test_script"; then
                echo "PASS" > "$result_file"
            else
                echo "FAIL" > "$result_file"
            fi
        ) &
        pids+=($!)
        scripts+=("$name")
    done

    # Wait for all remaining
    for pid in "${pids[@]}"; do
        wait "$pid" 2>/dev/null || true
    done

    # Collect results
    for name in "${scripts[@]}"; do
        local result_file="$results_dir/$name"
        if [ -f "$result_file" ] && [ "$(cat "$result_file")" = "PASS" ]; then
            passed=$((passed + 1))
        else
            failed=$((failed + 1))
            failed_names+=("$name")
        fi
    done

    rm -rf "$results_dir"
    print_summary $passed $failed "${failed_names[@]}"
}

# ============================================================================
# Summary
# ============================================================================
print_summary() {
    local passed=$1
    local failed=$2
    shift 2
    local failed_names=("$@")

    echo ""
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║                       QA SUITE SUMMARY                             ║"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    echo ""
    echo "  Total:   $TOTAL"
    echo "  Passed:  $passed"
    echo "  Failed:  $failed"
    echo ""

    if [ "$failed" -gt 0 ]; then
        echo "  Failed tests:"
        for name in "${failed_names[@]}"; do
            echo "    ❌ $name"
        done
        echo ""
        echo "❌ QA SUITE FAILED"
        exit 1
    else
        echo "✅ ALL QA TESTS PASSED"
        exit 0
    fi
}

# ============================================================================
# Main
# ============================================================================
SUITE_START=$(date +%s)

if [ "$PARALLEL" -gt 0 ]; then
    run_parallel
else
    run_sequential
fi
