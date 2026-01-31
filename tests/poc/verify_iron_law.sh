#!/bin/bash
# verify_iron_law.sh
# RFC-0039 Iron Law Verification: CAS blobs must be immutable.

THE_SOURCE="${VR_THE_SOURCE:-$HOME/.vrift/the_source}"

echo "--- Iron Law Verification: $THE_SOURCE ---"

if [ ! -d "$THE_SOURCE" ]; then
    echo "[SKIP] CAS directory $THE_SOURCE not found. Run an ingest first."
    exit 0
fi

# Find a random blob in CAS
BLOB=$(find "$THE_SOURCE" -type f -name "*.bin" | head -n 1)

if [ -z "$BLOB" ]; then
    echo "[SKIP] No blobs found in CAS."
    exit 0
fi

echo "Testing blob: $BLOB"

# 1. Test Write Protection (chmod 444)
echo "Attempting to overwrite with echo (non-root)..."
if echo "corrupt" > "$BLOB" 2>/dev/null; then
    echo "[FAIL] Managed to overwrite CAS blob using echo!"
    exit 1
else
    echo "[OK] Write denied as expected."
fi

# 2. Test Deletion Protection (uchg/immutable flag)
echo "Attempting to delete (non-root)..."
if rm "$BLOB" 2>/dev/null; then
    echo "[FAIL] Managed to delete CAS blob using rm!"
    exit 1
else
    echo "[OK] Delete denied as expected."
fi

# 3. Test Invariant: No Execute Bits
echo "Checking for execute bits..."
PERMS=$(stat -f "%Sp" "$BLOB" 2>/dev/null || stat -c "%A" "$BLOB")
if [[ "$PERMS" == *[x]* ]]; then
    echo "[FAIL] Execute bits found: $PERMS"
    exit 1
else
    echo "[OK] No execute bits found: $PERMS"
fi

echo "[SUCCESS] Iron Law Verification complete."
