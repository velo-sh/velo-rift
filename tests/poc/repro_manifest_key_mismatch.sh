#!/bin/bash
# repro_manifest_key_mismatch.sh
# Solidifies the bug where manifest keys include virtual prefixes that the shim strips.

set -e
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VELO_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_shim.dylib"

TEST_DIR=$(mktemp -d)
mkdir -p "$TEST_DIR/source"
echo "manifest content" > "$TEST_DIR/source/foo.txt"

# 1. Ingest with an explicit prefix
export VR_THE_SOURCE="$TEST_DIR/cas"
echo "--- Ingesting with prefix /myvirt ---"
"$VELO_BIN" ingest "$TEST_DIR/source" --prefix /myvirt

# 2. Check the manifest key using 'vrift status' if possible, or just proof via shim
# (Actually, let's just use the shim to prove it)

# 3. Start daemon
export VRIFT_MANIFEST="$TEST_DIR/source/.vrift/manifest.lmdb"
"$VRIFTD_BIN" start > "$TEST_DIR/daemon.log" 2>&1 &
sleep 2

# 4. Proof: Try to access the file via shim
# If the manifest lookup fails ('NOT FOUND' in logs), the key mismatch is proven.
echo "--- Accessing /myvirt/foo.txt ---"
# Compile arm64 cat (arm64e /bin/cat doesn't work with DYLD injection)
echo '#include <stdio.h>
int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        FILE *f = fopen(argv[i], "r"); if (!f) { perror(argv[i]); return 1; }
        int c; while ((c = fgetc(f)) != EOF) putchar(c); fclose(f);
    }
    return 0;
}' | cc -O2 -x c - -o "$TEST_DIR/cat"
codesign -s - -f "$TEST_DIR/cat" 2>/dev/null || true
DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 VRIFT_VFS_PREFIX="/myvirt" VRIFT_DEBUG=1 "$TEST_DIR/cat" /myvirt/foo.txt 2>&1 || true

echo ""
echo "--- Daemon Log (Lookup Result) ---"
grep "manifest lookup" "$TEST_DIR/daemon.log" || echo "No lookup logged (check shim debug output)"

# Clean up
pkill vriftd || true
rm -rf "$TEST_DIR"
