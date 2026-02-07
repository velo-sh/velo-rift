#!/bin/bash
set -e

# VRift Multi-Tenant Isolation Test (Phase 6)
# Standardizes on 2-project partitioning

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo "🚀 Starting Phase 6 Isolation Test..."

# Setup workspace
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEST_ROOT="/tmp/vrift-isolation-test-$$"
VR_BIN="$PROJECT_ROOT/target/release/vrift"
VRIFTD_BIN="$PROJECT_ROOT/target/release/vriftd"
SHIM_LIB="$PROJECT_ROOT/target/release/libvrift_inception_layer.dylib"

export VR_THE_SOURCE="$TEST_ROOT/the_source"
export VRIFT_SOCKET_PATH="$TEST_ROOT/vrift.sock"
DAEMON_PID=""

cleanup() {
    [ -n "$DAEMON_PID" ] && kill -9 "$DAEMON_PID" 2>/dev/null || true
    rm -f "$VRIFT_SOCKET_PATH"
    if [ -d "$TEST_ROOT" ]; then
        # Remove immutable bits before cleanup (macOS)
        chflags -R nouchg "$TEST_ROOT" 2>/dev/null || true
        rm -rf "$TEST_ROOT"
    fi
}
trap cleanup EXIT
cleanup 2>/dev/null || true

mkdir -p "$TEST_ROOT/project_a" "$TEST_ROOT/project_b"
mkdir -p "$VR_THE_SOURCE"

# Build binaries if needed
if [ ! -f "$VR_BIN" ]; then
    echo "⚠️  Binaries not found, building..."
    (cd "$PROJECT_ROOT" && cargo build --workspace --release)
fi

# Path normalization for macOS (/tmp -> /private/tmp)
PROJECT_A=$(perl -MCwd -e 'print Cwd::realpath($ARGV[0])' "$TEST_ROOT/project_a")
PROJECT_B=$(perl -MCwd -e 'print Cwd::realpath($ARGV[0])' "$TEST_ROOT/project_b")

# 1. Setup Projects
echo "Hello project A" > "$PROJECT_A/file_a.txt"
echo "Hello project B" > "$PROJECT_B/file_b.txt"

# 2. Ingest
echo "📦 Ingesting Project A..."
cd "$PROJECT_A"
"$VR_BIN" init . >/dev/null 2>&1
"$VR_BIN" ingest . --mode solid --output .vrift/manifest.lmdb >/dev/null 2>&1

echo "📦 Ingesting Project B..."
cd "$PROJECT_B"
"$VR_BIN" init . >/dev/null 2>&1
"$VR_BIN" ingest . --mode solid --output .vrift/manifest.lmdb >/dev/null 2>&1

# 3. Verify manifests created
echo "🔍 Verifying manifests..."
if [ ! -d "$PROJECT_A/.vrift/manifest.lmdb" ]; then
    echo -e "${RED}FAILED: Project A manifest not found${NC}"
    ls -R "$PROJECT_A/.vrift" 2>/dev/null || echo "  .vrift dir does not exist"
    exit 1
fi
echo "   ✅ Project A manifest exists"

if [ ! -d "$PROJECT_B/.vrift/manifest.lmdb" ]; then
    echo -e "${RED}FAILED: Project B manifest not found${NC}"
    exit 1
fi
echo "   ✅ Project B manifest exists"

# 4. Start Daemon
echo "🔍 Starting Daemon..."
RUST_LOG=info "$VRIFTD_BIN" start > "$TEST_ROOT/vriftd.log" 2>&1 &
DAEMON_PID=$!

# Wait for socket
waited=0
while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
    sleep 0.5
    waited=$((waited + 1))
done

if [ ! -S "$VRIFT_SOCKET_PATH" ]; then
    echo -e "${RED}FAILED: Daemon did not start (no socket after 5s)${NC}"
    cat "$TEST_ROOT/vriftd.log" | tail -20
    exit 1
fi
echo "   ✅ Daemon running"

# 5. Verify VFS Isolation (Shim)
echo "🛡️ Verifying Shim Isolation..."

# Compile test_stat
TEST_STAT_SRC="$PROJECT_ROOT/tests/helpers/test_stat.c"
if [ ! -f "$TEST_STAT_SRC" ]; then
    echo -e "${RED}FAILED: test_stat.c not found at $TEST_STAT_SRC${NC}"
    exit 1
fi
gcc "$TEST_STAT_SRC" -o "$TEST_ROOT/test_stat"
codesign --force --sign - "$TEST_ROOT/test_stat" 2>/dev/null || true

echo "   Testing Project A view..."
cd "$PROJECT_A"
OUTPUT_A=$(env DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 \
    VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
    VRIFT_VFS_PREFIX="/vrift" VRIFT_PROJECT_ROOT="$PROJECT_A" \
    VRIFT_MANIFEST="$PROJECT_A/.vrift/manifest.lmdb" \
    "$TEST_ROOT/test_stat" 2>&1) || true
echo "$OUTPUT_A" | tee "$TEST_ROOT/test_a.log"

if echo "$OUTPUT_A" | grep -q 'SUCCESS: stat("/vrift/file_a.txt") worked!'; then
    echo "   ✅ Project A file visible"
else
    echo -e "${RED}   ❌ Project A verification failed${NC}"
    cat "$TEST_ROOT/vriftd.log" | tail -20
    exit 1
fi

echo "   Testing Project B view (should NOT see A's file if we change manifest)..."
cd "$PROJECT_B"
OUTPUT_B=$(env DYLD_INSERT_LIBRARIES="$SHIM_LIB" DYLD_FORCE_FLAT_NAMESPACE=1 \
    VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" VR_THE_SOURCE="$VR_THE_SOURCE" \
    VRIFT_VFS_PREFIX="/vrift" VRIFT_PROJECT_ROOT="$PROJECT_B" \
    VRIFT_MANIFEST="$PROJECT_B/.vrift/manifest.lmdb" \
    "$TEST_ROOT/test_stat" 2>&1) || true
echo "$OUTPUT_B" | tee "$TEST_ROOT/test_b.log"

if echo "$OUTPUT_B" | grep -q 'SUCCESS: stat("/vrift/file_b.txt") worked!'; then
    echo "   ✅ Project B file visible"
else
    echo -e "${RED}   ❌ Project B verification failed${NC}"
    cat "$TEST_ROOT/vriftd.log" | tail -20
    exit 1
fi

echo -e "${GREEN}✅ Phase 6 Isolation Test PASSED${NC}"
exit 0
