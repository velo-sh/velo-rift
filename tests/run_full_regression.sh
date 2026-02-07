#!/bin/bash
# Full Regression Test Runner
# Runs all test suites: qa_v2, qa_v3, integration, extended, e2e, stress, poc
# Usage: bash tests/run_full_regression.sh
#
# Options (env vars):
#   TIMEOUT_SEC=120    Per-test timeout in seconds (default: 120)
#   SKIP_POC=1         Skip POC tests for a faster run
#   SKIP_BENCH=1       Skip benchmark tests

set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "$PROJECT_ROOT"

TIMEOUT_SEC="${TIMEOUT_SEC:-120}"
PASS=0; FAIL=0; TO=0; TOTAL=0
declare -a FAIL_LIST=()

run() {
    local t="$1"; local bn=$(basename "$t")
    # Skip benchmarks if requested
    if [ "${SKIP_BENCH:-0}" = "1" ] && [[ "$bn" == bench_* ]]; then return; fi
    TOTAL=$((TOTAL+1))
    local s=$(date +%s)
    bash "$t" > "/tmp/regression_${bn}.log" 2>&1 &
    local p=$!
    (sleep $TIMEOUT_SEC && kill -9 $p 2>/dev/null) &
    local w=$!
    wait $p 2>/dev/null; local ec=$?
    kill $w 2>/dev/null; wait $w 2>/dev/null
    local d=$(( $(date +%s) - s ))
    if [ $ec -eq 0 ]; then
        echo "  [PASS] $bn (${d}s)"; PASS=$((PASS+1))
    elif [ $ec -eq 137 ] && [ $d -ge $((TIMEOUT_SEC-2)) ]; then
        echo "  [TIMEOUT] $bn (${d}s)"; TO=$((TO+1)); FAIL_LIST+=("TIMEOUT|$bn")
    else
        echo "  [FAIL] $bn (${d}s, exit=$ec)"; FAIL=$((FAIL+1)); FAIL_LIST+=("FAIL|$bn|$ec")
        tail -3 "/tmp/regression_${bn}.log" | sed 's/^/    | /'
    fi
}

COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
echo "============================================"
echo "  FULL REGRESSION @ ${COMMIT}"
echo "  $(date '+%Y-%m-%d %H:%M:%S')"
echo "  Timeout: ${TIMEOUT_SEC}s per test"
echo "============================================"
echo ""

echo "--- qa_v2 ---"
for t in tests/qa_v2/test_*.sh tests/qa_v2/bench_*.sh; do [ -f "$t" ] && run "$t"; done

echo ""
echo "--- qa_v3 ---"
for t in tests/qa_v3/test_*.sh; do [ -f "$t" ] && run "$t"; done

echo ""
echo "--- integration ---"
for t in tests/integration/*.sh; do [ -f "$t" ] && run "$t"; done

echo ""
echo "--- extended ---"
for t in tests/extended/*.sh; do [ -f "$t" ] && run "$t"; done

echo ""
echo "--- e2e ---"
for t in tests/e2e/*.sh; do [ -f "$t" ] && run "$t"; done

echo ""
echo "--- stress ---"
for t in tests/stress/*.sh; do [ -f "$t" ] && run "$t"; done

if [ "${SKIP_POC:-0}" != "1" ]; then
    echo ""
    echo "--- poc ---"
    for t in tests/poc/*.sh; do
        bn=$(basename "$t")
        case "$bn" in run_*.sh) continue ;; esac
        run "$t"
    done
fi

echo ""
echo "--- top-level ---"
for t in tests/test_isolation.sh tests/scorched_earth_verification.sh; do [ -f "$t" ] && run "$t"; done

echo ""
echo "============================================"
echo "  RESULTS: $PASS PASS, $FAIL FAIL, $TO TIMEOUT (of $TOTAL)"
if [ $TOTAL -gt 0 ]; then
    echo "  Pass Rate: $(( PASS * 100 / TOTAL ))%"
fi
echo "============================================"
echo ""
if [ ${#FAIL_LIST[@]} -gt 0 ]; then
    echo "FAILURES:"
    for f in "${FAIL_LIST[@]}"; do echo "  $f"; done
fi
