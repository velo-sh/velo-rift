#!/bin/bash
# ==============================================================================
# Velo Rift POC: Flock Performance & Mutual Exclusion Test
# ==============================================================================

set -e

# Setup test directory
TEST_DIR=$(mktemp -d -t vrift_flock_test)
cd "$TEST_DIR"
LOCK_FILE="test.lock"

# Compile the helper
gcc -O2 /Users/antigravity/rust_source/vrift_qa/tests/poc/flock_test.c -o flock_test

echo "--- Testing Flock Mutual Exclusion ---"

# 1. Start process A to hold EXCLUSIVE lock for 3 seconds
echo "[$(date +%T)] Process A: Acquiring EX lock for 3s..."
./flock_test "$LOCK_FILE" 2 3000 &
PID_A=$!

# Give process A a moment to start and acquire the lock
sleep 0.5

# 2. Start process B to acquire EXCLUSIVE lock
echo "[$(date +%T)] Process B: Attempting to acquire EX lock..."
START_B=$(python3 -c "import time; print(int(time.time()*1000))")
./flock_test "$LOCK_FILE" 2 0 > /dev/null
END_B=$(python3 -c "import time; print(int(time.time()*1000))")

wait $PID_A
DURATION=$((END_B - START_B))

echo "[$(date +%T)] Process B: Acquired lock after ${DURATION}ms"

# 3. Verification
# It should have taken at least ~2s (3s minus the 0.5s sleep)
if [ "$DURATION" -ge 2000 ]; then
    echo "✅ PASS: Process B was correctly blocked by Process A's lock."
else
    echo "❌ FAIL: Process B was NOT blocked. Mutual exclusion failed or timing is off."
    exit 1
fi

echo "--- Test Complete ---"
rm -rf "$TEST_DIR"
