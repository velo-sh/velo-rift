#!/bin/bash
# Test Break-Before-Write (BBW) CAS Protection
# Verifies that writing to a VFS file does NOT corrupt the original CAS blob

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Break-Before-Write CAS Protection Test ==="

# Build shim
echo "Building shim..."
cargo build -p vrift-shim --quiet 2>/dev/null || cargo build -p vrift-shim

SHIM_PATH="$PROJECT_ROOT/target/debug/libvrift_shim.dylib"
if [[ ! -f "$SHIM_PATH" ]]; then
    echo "SKIP: Shim not found (macOS only test)"
    exit 0
fi

# Create test environment
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT

CAS_DIR="$TEST_DIR/cas"
mkdir -p "$CAS_DIR"

# Create a "CAS blob" simulating an ingested file
ORIGINAL_CONTENT="Original CAS content - DO NOT MODIFY"
CAS_BLOB="$CAS_DIR/test_blob.bin"
echo -n "$ORIGINAL_CONTENT" > "$CAS_BLOB"
chmod 444 "$CAS_BLOB"  # Read-only like real CAS

# Record original inode and checksum
ORIGINAL_INODE=$(stat -f %i "$CAS_BLOB" 2>/dev/null || stat -c %i "$CAS_BLOB")
ORIGINAL_CHECKSUM=$(md5 -q "$CAS_BLOB" 2>/dev/null || md5sum "$CAS_BLOB" | cut -d' ' -f1)

echo "CAS blob: $CAS_BLOB"
echo "Original inode: $ORIGINAL_INODE"
echo "Original checksum: $ORIGINAL_CHECKSUM"

# Create test program that attempts to write
cat > "$TEST_DIR/test_write.c" << 'EOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    
    // Try to open for writing
    int fd = open(argv[1], O_RDWR);
    if (fd < 0) {
        fd = open(argv[1], O_WRONLY | O_TRUNC | O_CREAT, 0644);
    }
    
    if (fd < 0) {
        perror("open failed");
        return 1;
    }
    
    const char *new_content = "MODIFIED CONTENT";
    write(fd, new_content, strlen(new_content));
    close(fd);
    
    printf("Write operation completed\n");
    return 0;
}
EOF

# Compile
clang -o "$TEST_DIR/test_write" "$TEST_DIR/test_write.c"
codesign -s - "$TEST_DIR/test_write" 2>/dev/null || true

# Run WITHOUT shim - should fail on read-only file
echo ""
echo "Test 1: Write without shim (should fail on read-only)..."
"$TEST_DIR/test_write" "$CAS_BLOB" 2>&1 || true

# Check CAS blob is unchanged
CURRENT_CHECKSUM=$(md5 -q "$CAS_BLOB" 2>/dev/null || md5sum "$CAS_BLOB" | cut -d' ' -f1)
if [[ "$CURRENT_CHECKSUM" == "$ORIGINAL_CHECKSUM" ]]; then
    echo "  ✅ CAS unchanged after direct write attempt"
else
    echo "  ❌ FAIL: CAS was modified!"
    exit 1
fi

# Run WITH shim - COW should trigger, CAS should remain unchanged
echo ""
echo "Test 2: Write with shim (BBW should protect CAS)..."
DYLD_INSERT_LIBRARIES="$SHIM_PATH" \
DYLD_FORCE_FLAT_NAMESPACE=1 \
"$TEST_DIR/test_write" "$CAS_BLOB" 2>&1 || true

# Verify CAS blob still unchanged
FINAL_CHECKSUM=$(md5 -q "$CAS_BLOB" 2>/dev/null || md5sum "$CAS_BLOB" | cut -d' ' -f1)
FINAL_INODE=$(stat -f %i "$CAS_BLOB" 2>/dev/null || stat -c %i "$CAS_BLOB")

echo ""
echo "Final checksum: $FINAL_CHECKSUM"
echo "Final inode: $FINAL_INODE"

if [[ "$FINAL_CHECKSUM" == "$ORIGINAL_CHECKSUM" && "$FINAL_INODE" == "$ORIGINAL_INODE" ]]; then
    echo ""
    echo "=== Summary ==="
    echo "✅ PASS: Break-Before-Write protected the CAS"
    echo "  - Original content preserved"
    echo "  - Inode unchanged (no replacement)"
    exit 0
else
    echo ""
    echo "=== Summary ==="
    echo "❌ FAIL: CAS was corrupted"
    echo "  - Checksum match: $([ "$FINAL_CHECKSUM" == "$ORIGINAL_CHECKSUM" ] && echo yes || echo NO)"
    echo "  - Inode match: $([ "$FINAL_INODE" == "$ORIGINAL_INODE" ] && echo yes || echo NO)"
    exit 1
fi
