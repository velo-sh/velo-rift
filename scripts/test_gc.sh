#!/bin/bash
set -e

# Setup paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VELO_BIN="$SCRIPT_DIR/../target/release/velo"
TEST_DIR=$(mktemp -d)

echo "Testing Velo GC in $TEST_DIR"

CAS_DIR="$TEST_DIR/cas"
MANIFEST_DIR="$TEST_DIR/manifests"
mkdir -p "$CAS_DIR"
mkdir -p "$MANIFEST_DIR"

# 1. Create content
mkdir -p "$TEST_DIR/used_dir"
echo "Used Content" > "$TEST_DIR/used_dir/file.txt"

mkdir -p "$TEST_DIR/garbage_dir"
echo "Garbage Content" > "$TEST_DIR/garbage_dir/file.txt"

# 2. Ingest "Used" content to create a manifest
$VELO_BIN --cas-root "$CAS_DIR" ingest "$TEST_DIR/used_dir" --output "$MANIFEST_DIR/manifest.bin" --prefix "test"

echo "Ingested used content."

# 3. Manually insert "Garbage" content
$VELO_BIN --cas-root "$CAS_DIR" ingest "$TEST_DIR/garbage_dir" --output "$TEST_DIR/garbage_manifest.bin" --prefix "garbage"
rm "$TEST_DIR/garbage_manifest.bin"

echo "Inserted garbage content and removed its manifest."

# 4. Verify CAS contains both
COUNT=$(find "$CAS_DIR" -type f | wc -l)
echo "CAS files found: $COUNT"

# 5. Run GC in dry Run
echo "Running GC (Dry Run)..."
OUTPUT=$($VELO_BIN --cas-root "$CAS_DIR" gc --manifests "$MANIFEST_DIR" --verbose)
echo "$OUTPUT"

if echo "$OUTPUT" | grep -q "Mode: DRY RUN"; then
    echo "Dry run confirmed."
else
    echo "Error: Not in dry run mode?"
    exit 1
fi

if echo "$OUTPUT" | grep -q "Garbage blobs:  1"; then
    echo "Correctly identified 1 garbage blob."
else
    echo "Error: Did not identify exactly 1 garbage blob."
    exit 1
fi

# 6. Run GC with Delete
echo "Running GC (Delete)..."
$VELO_BIN --cas-root "$CAS_DIR" gc --manifests "$MANIFEST_DIR" --delete --verbose

# 7. Verify Garbage is gone
FINAL_OUTPUT=$($VELO_BIN --cas-root "$CAS_DIR" gc --manifests "$MANIFEST_DIR")
if echo "$FINAL_OUTPUT" | grep -q "Garbage blobs:  0"; then
    echo "GC successful: 0 garbage blobs remaining."
else
    echo "Error: Garbage blobs still exist."
    exit 1
fi

echo "GC Test Passed!"
rm -rf "$TEST_DIR"
