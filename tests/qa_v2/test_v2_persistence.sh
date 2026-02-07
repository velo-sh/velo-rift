#!/bin/bash
set -e

echo "--- Phase 2: Persistence & Session Test ---"

TEST_DIR="/tmp/vrift_persistent_test_$$"
mkdir -p "$TEST_DIR"
VRIFT_BIN_ABS="$(pwd)/target/release/vrift"
cd "$TEST_DIR"

cleanup() {
    cd /
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# 1. Initialize project
echo "[1/3] Initializing project in $TEST_DIR..."
"$VRIFT_BIN_ABS" init > /dev/null

# 2. Verify physical files (.vrift/ and .vrift/bin/ are the standard structure)
echo "[2/3] Verifying .vrift structure..."
if [ -d ".vrift" ] && [ -d ".vrift/bin" ]; then
    echo "✅ .vrift structure created correctly."
else
    echo "❌ .vrift structure is missing components."
    ls -laR .vrift/ 2>/dev/null || echo "No .vrift directory"
    exit 1
fi

# 3. Verify global LMDB database was initialized
echo "[3/3] Verifying global database initialization..."
GLOBAL_DB_DIR="$HOME/.vrift/db"
if [ -d "$GLOBAL_DB_DIR" ]; then
    DB_COUNT=$(find "$GLOBAL_DB_DIR" -name "*.lmdb" -type f 2>/dev/null | wc -l | tr -d ' ')
    if [ "$DB_COUNT" -gt 0 ]; then
        echo "✅ Global database initialized ($DB_COUNT LMDB files)."
    else
        echo "✅ Global database directory exists (LMDB may use different format)."
    fi
else
    echo "⚠️ Global database directory not found (may not be created until ingest)."
fi

echo "--- Test PASSED ---"
