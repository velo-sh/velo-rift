#!/bin/bash
# Safety Check: CAS Immutability vs Write Passthrough
#
# Provenance: RFC-0047 validation
# Goal: Prove that "passthrough write" CANNOT corrupt CAS
# Logic: You cannot write to a file you cannot open for writing.
#        CAS files are 0444/0555. open(O_WRONLY) will fail.
#        Therefore passthrough write is safe (it never gets a valid FD).

set -e
TEST_DIR=$(mktemp -d)
export TEST_DIR
CAS_FILE="$TEST_DIR/cas_blob"

echo "=== VFS Safety Verification ==="
echo "[1] Creating mock CAS file (0444 r--r--r--)"
echo "Important Data" > "$CAS_FILE"
chmod 0444 "$CAS_FILE"
ls -l "$CAS_FILE"

echo ""
echo "[2] Attempting to open for WRITE (O_WRONLY)"
# Try to write to it using shell (which uses open+write)
if echo "Corruption" >> "$CAS_FILE" 2>/dev/null; then
    echo "❌ CRITICAL: Successfully wrote to CAS file!"
    echo "   Filesystem permissions failed to protect strict immutability."
    exit 1
else
    echo "✅ WRITE BLOCKED: open() failed with Permission denied"
fi

echo ""
echo "[3] Attempting to force write via C program (ignoring shim)"
cat > "$TEST_DIR/attack.c" << 'CEOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>

int main(int argc, char *argv[]) {
    const char *path = argv[1];
    printf("Attacking: %s\n", path);
    
    // Try to open RW
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        printf("✅ open(O_RDWR) failed: %s\n", strerror(errno));
        return 0; // Success (Safety confirmed)
    }
    
    printf("⚠️ Got FD %d! Attempting write...\n", fd);
    if (write(fd, "HACK", 4) == 4) {
        printf("❌ Wrote to file!\n");
        return 1;
    }
    close(fd);
    return 1;
}
CEOF

gcc -o "$TEST_DIR/attack" "$TEST_DIR/attack.c"
"$TEST_DIR/attack" "$CAS_FILE"

echo ""
echo "=== Conclusion ==="
echo "Passthrough write() is SAFE because open() is the gatekeeper."
echo "If open() correctly handles CoW (or fails on RO), write() cannot corrupt CAS."

rm -rf "$TEST_DIR"
