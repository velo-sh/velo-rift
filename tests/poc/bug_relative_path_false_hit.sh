#!/bin/bash
# Proof of Failure: Relative Path False Positive Hit
# Demonstrates that the shim incorrectly resolves relative paths against the project root
# even when the process is in a different current working directory.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT

VRIFT_BIN="$PROJECT_ROOT/target/release/vrift"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

# 1. Setup Project
mkdir -p "$WORK_DIR/project"
touch "$WORK_DIR/project/secret.txt"
echo "VIRTUAL CONTENT" > "$WORK_DIR/project/secret.txt"

# 2. Setup External Directory
mkdir -p "$WORK_DIR/external"
echo "REAL EXTERNAL CONTENT" > "$WORK_DIR/external/secret.txt"

# 3. Build Test Tool (cat replacement)
cat <<EOF > "$WORK_DIR/my_cat.c"
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
int main(int argc, char *argv[]) {
    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) { return 1; }
    char buf[1024];
    ssize_t n = read(fd, buf, 1023);
    if (n > 0) { buf[n] = 0; printf("%s", buf); }
    close(fd);
    return 0;
}
EOF
gcc -o "$WORK_DIR/my_cat" "$WORK_DIR/my_cat.c"

# 4. Inception Mode
export VRIFT_VFS_PREFIX="$WORK_DIR/project"
export VRIFT_MANIFEST="$WORK_DIR/project/.vrift/manifest.lmdb" # Not strictly needed for metadata-only test but good practice
export DYLD_INSERT_LIBRARIES="$SHIM_LIB"
export DYLD_FORCE_FLAT_NAMESPACE=1

# We need a dummy manifest for the shim to think it's in VFS
# Or just use the fact that resolve_path only checks prefix if configured.

echo "🧪 Case: Relative path in EXTERNAL directory"
cd "$WORK_DIR/external"
echo "   CWD: $(pwd)"
echo "   File exists: ./secret.txt (Content: REAL EXTERNAL CONTENT)"

# Run my_cat
OUT=$("$WORK_DIR/my_cat" "secret.txt" 2>&1)

echo "   Result: $OUT"

if echo "$OUT" | grep -q "VIRTUAL CONTENT"; then
    echo "💥 SLAP: False Positive Hit!"
    echo "   The shim resolved 'secret.txt' relative to the project root instead of CWD."
elif echo "$OUT" | grep -q "REAL EXTERNAL CONTENT"; then
    echo "✅ Success: Shim correctly ignored external relative path."
fi
