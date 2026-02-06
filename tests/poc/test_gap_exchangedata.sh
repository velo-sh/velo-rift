#!/bin/bash
# ============================================================================
# POC Test: exchangedata bypass (macOS only)
# ============================================================================
# This test proves that exchangedata can atomically swap VFS file contents
# with external files, bypassing the VFS mutation perimeter.
# ============================================================================

set -e

# macOS only
if [ "$(uname)" != "Darwin" ]; then
    echo "⏭️  SKIPPED: exchangedata is macOS-only"
    exit 0
fi

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m'

echo "================================================================"
echo "🧪 POC Test: exchangedata VFS Bypass (macOS)"
echo "================================================================"

WORK_DIR=$(mktemp -d)
mkdir -p "$WORK_DIR/project" "$WORK_DIR/external" "$WORK_DIR/cas"

# Create VFS file and external file with different content
echo "VFS_ORIGINAL_CONTENT" > "$WORK_DIR/project/vfs_file.txt"
echo "EXTERNAL_CONTENT" > "$WORK_DIR/external/ext_file.txt"

# Ingest into VFS
"$VRIFT_BIN" --the-source-root "$WORK_DIR/cas" ingest "$WORK_DIR/project" --mode solid > /dev/null 2>&1

# Create test program using exchangedata
cat > "$WORK_DIR/test_exchangedata.c" << 'EOF'
#include <stdio.h>
#include <string.h>
#include <errno.h>

// exchangedata is a macOS-specific syscall
extern int exchangedata(const char *path1, const char *path2, unsigned int options);

int main(int argc, char *argv[]) {
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <vfs_file> <external_file>\n", argv[0]);
        return 1;
    }
    
    printf("Attempting to swap: %s <-> %s\n", argv[1], argv[2]);
    
    // Try to atomically swap file contents
    int ret = exchangedata(argv[1], argv[2], 0);
    
    if (ret == 0) {
        printf("exchangedata succeeded - ATOMIC SWAP COMPLETED\n");
        return 0;  // Bypass confirmed
    } else {
        printf("exchangedata failed: %s (errno=%d)\n", strerror(errno), errno);
        return 1;  // Protected or unsupported
    }
}
EOF

clang -o "$WORK_DIR/test_exchangedata" "$WORK_DIR/test_exchangedata.c" 2>/dev/null || {
    echo -e "${YELLOW}⚠️  exchangedata not available in SDK, test cannot run${NC}"
    rm -rf "$WORK_DIR"
    exit 0
}

# Export VFS environment
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$WORK_DIR/project"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

# Record original content
VFS_BEFORE=$(cat "$WORK_DIR/project/vfs_file.txt")
EXT_BEFORE=$(cat "$WORK_DIR/external/ext_file.txt")

echo "Before swap:"
echo "  VFS file: $VFS_BEFORE"
echo "  External: $EXT_BEFORE"

# Run test
echo ""
echo "Running exchangedata test with VFS shim..."
"$WORK_DIR/test_exchangedata" "$WORK_DIR/project/vfs_file.txt" "$WORK_DIR/external/ext_file.txt" || true

# Check if swap occurred
VFS_AFTER=$(cat "$WORK_DIR/project/vfs_file.txt")
EXT_AFTER=$(cat "$WORK_DIR/external/ext_file.txt")

echo ""
echo "After swap attempt:"
echo "  VFS file: $VFS_AFTER"
echo "  External: $EXT_AFTER"

if [ "$VFS_AFTER" = "$EXT_BEFORE" ] && [ "$EXT_AFTER" = "$VFS_BEFORE" ]; then
    echo -e "${RED}❌ GAP CONFIRMED: exchangedata swapped VFS file content!${NC}"
    EXIT_CODE=0
elif [ "$VFS_AFTER" = "$VFS_BEFORE" ]; then
    echo -e "${GREEN}✅ PROTECTED: VFS file content unchanged${NC}"
    EXIT_CODE=1
else
    echo -e "${YELLOW}⚠️  UNEXPECTED: Content changed but not swapped${NC}"
    EXIT_CODE=2
fi

# Cleanup
rm -rf "$WORK_DIR"
echo "================================================================"
exit $EXIT_CODE
