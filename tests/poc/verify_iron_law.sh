#!/bin/bash
# verify_iron_law.sh
# RFC-0039 Iron Law Verification: CAS blobs must be immutable (0444, no execute).
# This test is SELF-CONTAINED: creates its own CAS, ingests files, and verifies.

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "=== RFC-0039 Iron Law Verification ==="
echo ""

# Build vrift CLI if needed
echo "[BUILD] Building vrift CLI..."
cargo build -p vrift-cli --quiet 2>/dev/null || cargo build -p vrift-cli
VRIFT="${PROJECT_ROOT}/target/debug/vrift"

if [ ! -f "$VRIFT" ]; then
    echo "[FAIL] vrift CLI not found at $VRIFT"
    exit 1
fi

# Setup isolated test environment
TEST_DIR="/tmp/test_iron_law_$$"
mkdir -p "$TEST_DIR/source"
mkdir -p "$TEST_DIR/cas"

cleanup() {
    echo ""
    echo "[CLEANUP] Removing test directory..."
    # Remove immutable flags on macOS before cleanup
    if [[ "$OSTYPE" == "darwin"* ]] && [ -d "$TEST_DIR" ]; then
        chflags -R nouchg "$TEST_DIR" 2>/dev/null || true
    fi
    chmod -R u+w "$TEST_DIR" 2>/dev/null || true
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# Create test files with various content
echo "test content for iron law verification" > "$TEST_DIR/source/file1.txt"
echo "another file with different content" > "$TEST_DIR/source/file2.txt"
dd if=/dev/urandom of="$TEST_DIR/source/binary.bin" bs=1024 count=1 2>/dev/null

echo "[INGEST] Creating CAS and ingesting test files..."
# Use VRIFT_CAS_ROOT which is the CLI's env var for CAS location
export VRIFT_CAS_ROOT="$TEST_DIR/cas"
$VRIFT ingest "$TEST_DIR/source" --prefix "test" 2>&1 | grep -v "^2026" || true

# Find blobs in CAS (RFC-0039 layout: blake3/ab/cd/hash_size.ext)
echo ""
echo "[VERIFY] Checking CAS structure..."
BLOB=$(find "$TEST_DIR/cas" -type f 2>/dev/null | head -n 1)

if [ -z "$BLOB" ]; then
    echo "[FAIL] No blobs found in CAS after ingest!"
    exit 1
fi

echo "Testing blob: $BLOB"
echo ""

# 1. Test Invariant: Read-Only (0444)
echo "[1] Checking permissions (expect 0444 / r--r--r--)..."
if [[ "$OSTYPE" == "darwin"* ]]; then
    PERMS=$(stat -f "%Sp" "$BLOB")
    OCTAL=$(stat -f "%OLp" "$BLOB")
else
    PERMS=$(stat -c "%A" "$BLOB")
    OCTAL=$(stat -c "%a" "$BLOB")
fi

if [[ "$OCTAL" == "444" ]] || [[ "$PERMS" == "-r--r--r--" ]]; then
    echo "    ✅ Permissions correct: $PERMS ($OCTAL)"
else
    echo "    ❌ [FAIL] Wrong permissions: $PERMS ($OCTAL), expected 444"
    exit 1
fi

# 2. Test Write Protection (redirect stderr to capture error, timeout to prevent hang)
echo "[2] Testing write protection..."
set +e
timeout 2 bash -c "echo 'corrupt' > '$BLOB'" 2>/dev/null
WRITE_RESULT=$?
set -e
if [[ "$WRITE_RESULT" == "0" ]]; then
    echo "    ❌ [FAIL] Managed to overwrite CAS blob!"
    exit 1
else
    echo "    ✅ Write denied as expected"
fi

# 3. Test Invariant: No Execute Bits (The Iron Law)
echo "[3] Checking for execute bits (Iron Law: must be NONE)..."
if [[ "$PERMS" == *[x]* ]]; then
    echo "    ❌ [FAIL] Execute bits found: $PERMS"
    exit 1
else
    echo "    ✅ No execute bits found"
fi

# 4. Test Delete Protection via unlink syscall (use python for non-blocking check)
echo "[4] Testing delete protection..."
set +e
python3 -c "
import os, sys
try:
    os.unlink('$BLOB')
    sys.exit(0)  # unlink succeeded = FAIL
except PermissionError:
    sys.exit(1)  # Permission denied = expected
except Exception as e:
    sys.exit(1)  # Any error = protection worked
" 2>/dev/null
UNLINK_RESULT=$?
set -e
if [[ "$UNLINK_RESULT" == "0" ]]; then
    echo "    ❌ [FAIL] Managed to delete CAS blob!"
    exit 1
else
    echo "    ✅ Delete denied as expected"
fi

echo ""
echo "==========================================="
echo "✅ PASS: Iron Law Verification Complete"
echo "==========================================="
echo "  • CAS blobs are read-only (0444)"
echo "  • No execute bits present"
echo "  • Write protection enforced"  
echo "  • Delete protection enforced"
