#!/bin/bash
# ============================================================================
# POC Test: readlinkat bypass via dirfd
# ============================================================================
# This test proves that readlinkat can read symlink targets in VFS
# using a directory FD, potentially bypassing path-based checks.
# ============================================================================

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo "================================================================"
echo "🧪 POC Test: readlinkat VFS Dirfd Resolution"
echo "================================================================"

WORK_DIR=$(mktemp -d)
mkdir -p "$WORK_DIR/project" "$WORK_DIR/cas"
echo "real content" > "$WORK_DIR/project/real.txt"
ln -s real.txt "$WORK_DIR/project/link.txt"

# Ingest into VFS
"$VRIFT_BIN" --the-source-root "$WORK_DIR/cas" ingest "$WORK_DIR/project" --mode solid > /dev/null 2>&1

# Create test program
cat > "$WORK_DIR/test_readlinkat.c" << 'EOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>

int main(int argc, char *argv[]) {
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <dir> <symlink_name>\n", argv[0]);
        return 1;
    }
    
    // Open directory as FD
    int dirfd = open(argv[1], O_RDONLY | O_DIRECTORY);
    if (dirfd < 0) {
        fprintf(stderr, "open dir failed: %s\n", strerror(errno));
        return 1;
    }
    
    // Use readlinkat with dirfd
    char buf[256];
    ssize_t len = readlinkat(dirfd, argv[2], buf, sizeof(buf) - 1);
    close(dirfd);
    
    if (len > 0) {
        buf[len] = '\0';
        printf("readlinkat resolved: %s -> %s\n", argv[2], buf);
        return 0;  // Successfully read via dirfd
    } else {
        printf("readlinkat failed: %s\n", strerror(errno));
        return 1;
    }
}
EOF

clang -o "$WORK_DIR/test_readlinkat" "$WORK_DIR/test_readlinkat.c"

# Export VFS environment
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$WORK_DIR/project"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

# Run test
echo ""
echo "Running readlinkat test with VFS shim..."
if "$WORK_DIR/test_readlinkat" "$WORK_DIR/project" "link.txt"; then
    echo -e "${GREEN}✅ readlinkat works correctly with VFS${NC}"
    EXIT_CODE=0
else
    echo -e "${RED}❌ readlinkat failed to resolve VFS symlink via dirfd${NC}"
    EXIT_CODE=1
fi

# Cleanup
rm -rf "$WORK_DIR"
echo "================================================================"
exit $EXIT_CODE
