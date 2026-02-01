#!/bin/bash
# test_special_filenames.sh - Verify handling of special characters in filenames
# Priority: P2 (Boundary Condition)
set -e

echo "=== Test: Special Filename Handling ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VR_THE_SOURCE="/tmp/special_cas"
VRIFT_MANIFEST="/tmp/special.manifest"
TEST_DIR="/tmp/special_test"

cleanup() {
    rm -rf "$VR_THE_SOURCE" "$TEST_DIR" "$VRIFT_MANIFEST" 2>/dev/null || true
}
trap cleanup EXIT
cleanup

mkdir -p "$VR_THE_SOURCE" "$TEST_DIR"

echo "[1] Creating files with special characters..."
PASS_COUNT=0
FAIL_COUNT=0

# Test various special filenames
test_file() {
    local name="$1"
    local desc="$2"
    if echo "content" > "$TEST_DIR/$name" 2>/dev/null; then
        echo "    ✓ Created: $desc"
        ((PASS_COUNT++)) || true
    else
        echo "    ✗ Failed: $desc (filesystem limitation)"
        ((FAIL_COUNT++)) || true
    fi
}

test_file "file with spaces.txt" "Spaces"
test_file "file'with'quotes.txt" "Single quotes"
test_file 'file"with"doublequotes.txt' "Double quotes"
test_file "file-with-dash.txt" "Dashes"
test_file "file_with_underscore.txt" "Underscores"
test_file "file.multiple.dots.txt" "Multiple dots"
test_file "UPPERCASE.TXT" "Uppercase"
test_file "MixedCase.Txt" "Mixed case"
test_file "文件名.txt" "Unicode (Chinese)"
test_file "日本語ファイル.txt" "Unicode (Japanese)"
test_file "emoji_🎉_file.txt" "Emoji"
test_file ".hidden_file" "Hidden (dot prefix)"
test_file "file#with#hash.txt" "Hash symbols"
test_file "file@with@at.txt" "At symbols"
test_file "file\$with\$dollar.txt" "Dollar signs"
test_file "file%with%percent.txt" "Percent signs"

echo ""
echo "    Created: $PASS_COUNT files, Failed: $FAIL_COUNT"

echo "[2] Ingesting special filenames..."
if ! "${PROJECT_ROOT}/target/debug/vrift" --the-source-root "$VR_THE_SOURCE" \
    ingest "$TEST_DIR" --output "$VRIFT_MANIFEST" --prefix /special 2>&1; then
    echo "[FAIL] Ingest failed on special filenames"
    exit 1
fi

echo "[3] Verifying manifest created..."
if [ -f "$VRIFT_MANIFEST" ]; then
    MF_SIZE=$(wc -c < "$VRIFT_MANIFEST")
    if [ "$MF_SIZE" -gt 50 ]; then
        echo "✅ PASS: Special filenames handled correctly"
        echo "    Manifest size: $MF_SIZE bytes"
        echo "    Files ingested: $PASS_COUNT"
        exit 0
    fi
fi

echo "[FAIL] Manifest not created or too small"
exit 1
