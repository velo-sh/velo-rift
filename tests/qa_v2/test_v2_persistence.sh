#!/bin/bash
set -e

echo "--- Phase 2: Persistence & Session Test ---"

TEST_DIR="/tmp/vrift_persistent_test_$$"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"
VRIFT_BIN="/Users/antigravity/rust_source/vrift_qa/target/release/vrift"

# 1. Initialize project
echo "[1/4] Initializing project in $TEST_DIR..."
# 'vrift init' outputs shell script, we ignore stdout but check side effects
$VRIFT_BIN init > /dev/null

# 2. Verify physical files
echo "[2/4] Verifying .vrift structure..."
if [ -d ".vrift" ] && [ -d ".vrift/bin" ] && [ -d ".vrift/manifest.lmdb" ]; then
    echo "✅ .vrift structure created correctly."
else
    echo "❌ .vrift structure is missing components."
    exit 1
fi

if [ -f ".vrift/session.json" ]; then
    echo "✅ .vrift/session.json exists."
    cat ".vrift/session.json"
else
    echo "❌ .vrift/session.json NOT found."
    exit 1
fi

# 3. Verify status output
echo "[3/4] Checking 'vrift status -s'..."
$VRIFT_BIN status -s | tee status_s.log
if grep -q "Session: ● \[Solid\] Active" status_s.log; then
    echo "✅ Session reported as Active and Solid."
else
    echo "❌ Session status mismatch."
    exit 1
fi

# 4. Verify persistence after wake
echo "[4/4] Testing persistence after wake..."
$VRIFT_BIN wake > /dev/null
$VRIFT_BIN status -s > status_wake.log
if grep -q "Active" status_wake.log; then
     echo "❌ Session still reported as Active after wake."
     exit 1
else
     echo "✅ Session reported as Inactive after wake."
fi

# Cleanup
cd /Users/antigravity/rust_source/vrift_qa
# rm -rf "$TEST_DIR"
echo "--- Test PASSED ---"
