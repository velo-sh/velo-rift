#!/bin/bash
# run_sprint1_validation.sh
#
# Master script to run all Sprint 1 M5 E2E validation tests

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}╔═══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║           Sprint 1 M5: E2E Validation Suite                   ║${NC}"
echo -e "${CYAN}╚═══════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Track results
PASSED=0
FAILED=0
SKIPPED=0

run_test() {
    local test_name=$1
    local test_script=$2
    local timeout_secs=${3:-120}  # Default 120s timeout
    
    echo -e "${YELLOW}━━━ Running: $test_name (timeout: ${timeout_secs}s) ━━━${NC}"
    
    if [ ! -f "$test_script" ]; then
        echo -e "${YELLOW}SKIP: $test_script not found${NC}"
        ((SKIPPED++))
        return
    fi
    
    chmod +x "$test_script"
    
    # Use perl alarm for cross-platform timeout (works on macOS and Linux)
    if perl -e "alarm $timeout_secs; exec @ARGV" "$test_script"; then
        echo -e "${GREEN}✓ $test_name: PASSED${NC}"
        ((PASSED++))
    else
        local exit_code=$?
        if [ $exit_code -eq 142 ]; then
            echo -e "${RED}✗ $test_name: TIMEOUT (${timeout_secs}s)${NC}"
        else
            echo -e "${RED}✗ $test_name: FAILED (exit $exit_code)${NC}"
        fi
        ((FAILED++))
    fi
    echo ""
}

# Run tests
run_test "test_rustc_single_file" "$SCRIPT_DIR/test_rustc_single_file.sh"
run_test "test_cargo_incremental" "$SCRIPT_DIR/test_cargo_incremental.sh"
run_test "test_concurrent_writers" "$SCRIPT_DIR/test_concurrent_writers.sh"

# Summary
echo -e "${CYAN}╔═══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║                     Test Summary                              ║${NC}"
echo -e "${CYAN}╠═══════════════════════════════════════════════════════════════╣${NC}"
printf "${CYAN}║${NC}  ${GREEN}Passed: %-3d${NC}  ${RED}Failed: %-3d${NC}  ${YELLOW}Skipped: %-3d${NC}               ${CYAN}║${NC}\n" $PASSED $FAILED $SKIPPED
echo -e "${CYAN}╚═══════════════════════════════════════════════════════════════╝${NC}"

if [ $FAILED -gt 0 ]; then
    echo -e "${RED}Sprint 1 M5 Validation: FAILED${NC}"
    exit 1
fi

echo -e "${GREEN}Sprint 1 M5 Validation: PASSED${NC}"
exit 0
