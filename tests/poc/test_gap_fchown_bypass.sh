#!/bin/bash
# ============================================================================
# POC Test: fchown/fchownat bypass via FD
# ============================================================================
# This test proves that fchown can modify ownership of VFS files via FD
# even when the VFS mutation perimeter should block direct path-based chown.
# ============================================================================

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m'

echo "================================================================"
echo "🧪 POC Test: fchown/fchownat VFS Bypass"
echo "================================================================"

WORK_DIR=$(mktemp -d)
mkdir -p "$WORK_DIR/project" "$WORK_DIR/cas"
echo "test content" > "$WORK_DIR/project/target.txt"

# Ingest into VFS
"$VRIFT_BIN" --the-source-root "$WORK_DIR/cas" ingest "$WORK_DIR/project" --mode solid > /dev/null 2>&1

# Create test program
cat > "$WORK_DIR/test_fchown.c" << 'EOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <errno.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <file>\n", argv[0]);
        return 1;
    }
    
    // Get original owner
    struct stat st_before;
    if (stat(argv[1], &st_before) != 0) {
        fprintf(stderr, "stat failed: %s\n", strerror(errno));
        return 1;
    }
    printf("Before: uid=%d, gid=%d\n", st_before.st_uid, st_before.st_gid);
    
    // Open file
    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) {
        fprintf(stderr, "open failed: %s\n", strerror(errno));
        return 1;
    }
    
    // Try fchown via FD (set to same owner - should still be blocked if protected)
    int ret = fchown(fd, st_before.st_uid, st_before.st_gid);
    if (ret == 0) {
        printf("fchown succeeded - VFS BYPASS CONFIRMED\n");
        close(fd);
        return 0;  // Bypass exists
    } else {
        printf("fchown blocked: %s\n", strerror(errno));
        close(fd);
        return 1;  // Properly protected
    }
}
EOF

clang -o "$WORK_DIR/test_fchown" "$WORK_DIR/test_fchown.c"

# Export VFS environment
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb"
export VRIFT_PROJECT_ROOT="$WORK_DIR/project"
export VRIFT_VFS_PREFIX="$WORK_DIR/project"
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

# Run test
echo ""
echo "Running fchown test with VFS shim..."
if "$WORK_DIR/test_fchown" "$WORK_DIR/project/target.txt"; then
    echo -e "${RED}❌ GAP CONFIRMED: fchown bypasses VFS protection via FD${NC}"
    EXIT_CODE=0
else
    echo -e "${GREEN}✅ PROTECTED: fchown is blocked by VFS${NC}"
    EXIT_CODE=1
fi

# Cleanup
rm -rf "$WORK_DIR"
echo "================================================================"
exit $EXIT_CODE
