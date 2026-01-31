#!/bin/bash
set -e

# Setup paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VELO_BIN="$SCRIPT_DIR/../target/debug/vrift"
if [ ! -f "$VELO_BIN" ]; then
    VELO_BIN="$SCRIPT_DIR/../target/release/vrift"
fi

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
$VELO_BIN --the-source-root "$CAS_DIR" ingest "$TEST_DIR/used_dir" --output "$MANIFEST_DIR/manifest.bin"

echo "Ingested used content (Solid Mode)."

# 3. Manually insert "Garbage" content (ingest and then throw away manifest)
$VELO_BIN --the-source-root "$CAS_DIR" ingest "$TEST_DIR/garbage_dir" --output "$TEST_DIR/garbage_manifest.bin"
rm "$TEST_DIR/garbage_manifest.bin"

echo "Inserted garbage content and removed its manifest."

# 4. Verify CAS contains both
COUNT=$(find "$CAS_DIR" -type f | wc -l)
echo "CAS files found: $COUNT"

# 5. Run GC in dry Run
echo "Running GC (Dry Run)..."
OUTPUT=$($VELO_BIN --the-source-root "$CAS_DIR" gc --manifest "$MANIFEST_DIR/manifest.bin")
echo "$OUTPUT"

if echo "$OUTPUT" | grep -q "Orphaned:      1"; then
    echo "Correctly identified 1 garbage blob."
else
    echo "Error: Did not identify exactly 1 garbage blob."
    exit 1
fi

# 6. Run GC with Delete
echo "Running GC (Delete)..."
$VELO_BIN --the-source-root "$CAS_DIR" gc --manifest "$MANIFEST_DIR/manifest.bin" --delete --yes

# 7. Verify Garbage is gone
FINAL_OUTPUT=$($VELO_BIN --the-source-root "$CAS_DIR" gc --manifest "$MANIFEST_DIR/manifest.bin")
if echo "$FINAL_OUTPUT" | grep -q "Orphaned:      0"; then
    echo "GC successful: 0 garbage blobs remaining."
else
    echo "Error: Garbage blobs still exist."
    exit 1
fi

# 8. Test Phantom Mode Ingest
echo "Testing Phantom Mode Ingest (RFC-0039)..."
# Use a fresh CAS and manifest to isolate Phantom mode demo
PHANTOM_CAS="$TEST_DIR/cas_phantom"
PHANTOM_MAN_DIR="$TEST_DIR/manifests_phantom"
mkdir -p "$PHANTOM_CAS" "$PHANTOM_MAN_DIR"

mkdir -p "$TEST_DIR/phantom_dir"
echo "Phantom Content" > "$TEST_DIR/phantom_dir/pfile.txt"

# Ingest with Phantom Mode (renames file into CAS)
$VELO_BIN --the-source-root "$PHANTOM_CAS" ingest "$TEST_DIR/phantom_dir" --mode phantom --output "$PHANTOM_MAN_DIR/phantom.bin"

# Verify source file is MOVED (should not exist in phantom_dir)
if [ -f "$TEST_DIR/phantom_dir/pfile.txt" ]; then
    echo "Error: Phantom mode did not move the file!"
    exit 1
fi

# Verify CAS has the file
COUNT=$(find "$PHANTOM_CAS" -type f | wc -l)
if [ "$COUNT" -eq 0 ]; then
    echo "Error: CAS is empty after phantom ingest!"
    exit 1
fi

echo "Phantom ingest successful: Source file moved to CAS."

# 9. Verify GC preserves phantom blobs
echo "Running GC to verify phantom blob preservation..."
FINAL_PHANTOM=$($VELO_BIN --the-source-root "$PHANTOM_CAS" gc --manifest "$PHANTOM_MAN_DIR/phantom.bin")
if echo "$FINAL_PHANTOM" | grep -q "Orphaned:      0"; then
    echo "GC correctly preserved phantom blobs."
else
    echo "Error: Phantom blob identified as garbage!"
    echo "$FINAL_PHANTOM"
    exit 1
fi

echo "GC + Phantom Mode Test Passed!"
chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
rm -rf "$TEST_DIR"
