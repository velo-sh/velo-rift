#!/bin/bash
# Reliable Test Runner for macOS
# Uses perl for timeout since macOS lacks GNU timeout

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TIMEOUT_SEC=30

run_with_timeout() {
    local script="$1"
    perl -e 'alarm shift; exec @ARGV' "$TIMEOUT_SEC" bash "$script" >/dev/null 2>&1
    return $?
}

passed=0
failed=0
timeout_count=0

echo "=== Running All Tests (${TIMEOUT_SEC}s timeout each) ==="

for t in tests/poc/test_*.sh; do
    name=$(basename "$t")
    run_with_timeout "$t"
    code=$?
    
    if [ $code -eq 0 ]; then
        passed=$((passed+1))
    elif [ $code -eq 142 ]; then  # SIGALRM = 14, exit code 128+14=142
        timeout_count=$((timeout_count+1))
        echo "⏱️ TIMEOUT: $name"
    else
        failed=$((failed+1))
    fi
done

echo ""
echo "=== Results ==="
echo "✅ Passed: $passed"
echo "❌ Failed: $failed"
echo "⏱️ Timeout: $timeout_count"
echo "Total: $((passed + failed + timeout_count))"
