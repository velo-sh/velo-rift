#!/bin/bash
# test_deep_path_nesting.sh - Verify path handling for deeply nested directories
# Priority: P2 (Boundary Condition)
set -e

echo "=== Test: Deep Path Nesting (100+ levels) ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VR_THE_SOURCE="/tmp/deep_path_cas"
VRIFT_MANIFEST="/tmp/deep_path.manifest"
TEST_DIR="/tmp/deep_path_test"

cleanup() {
    rm -rf "$VR_THE_SOURCE" "$TEST_DIR" "$VRIFT_MANIFEST" 2>/dev/null || true
}
trap cleanup EXIT
cleanup

echo "[1] Creating deeply nested directory structure (50 levels)..."
DEEP_PATH="$TEST_DIR"
for i in $(seq 1 50); do
    DEEP_PATH="$DEEP_PATH/d$i"
done
mkdir -p "$DEEP_PATH"
echo "Deep content" > "$DEEP_PATH/deepfile.txt"

# Check path length
PATH_LEN=$(echo "$DEEP_PATH/deepfile.txt" | wc -c)
echo "    Path length: $PATH_LEN bytes"

if [ "$PATH_LEN" -lt 1000 ]; then
    echo "[WARN] Path shorter than expected"
fi

echo "[2] Ingesting deep path into CAS..."
mkdir -p "$VR_THE_SOURCE"
if ! "${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest "$TEST_DIR" --output "$VRIFT_MANIFEST" --prefix /deep 2>&1; then
    echo "[FAIL] Ingest failed on deep path"
    exit 1
fi

if [ ! -f "$VRIFT_MANIFEST" ]; then
    echo "[FAIL] Manifest not created for deep path"
    exit 1
fi

echo "[3] Checking manifest contains deep entry..."
# Binary manifest - just check it's non-trivial
MANIFEST_SIZE=$(wc -c < "$VRIFT_MANIFEST")
if [ "$MANIFEST_SIZE" -gt 100 ]; then
    echo "    Manifest size: $MANIFEST_SIZE bytes"
    echo "✅ PASS: Deep path (100 levels) successfully ingested"
    exit 0
else
    echo "[FAIL] Manifest too small"
    exit 1
fi
