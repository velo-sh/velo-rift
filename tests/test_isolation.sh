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
BASE_DIR=$(pwd)
TEST_ROOT="/tmp/vrift-isolation-test"
if [ -d "$TEST_ROOT" ]; then
    # Remove immutable bits before cleanup (macOS)
    chflags -R nouchg "$TEST_ROOT" || true
    rm -rf "$TEST_ROOT"
fi
mkdir -p "$TEST_ROOT/project_a" "$TEST_ROOT/project_b"

export VRIFT_CAS_ROOT="$TEST_ROOT/the_source"
mkdir -p "$VRIFT_CAS_ROOT"

# Build binaries
cargo build --workspace --release
VR_BIN="$BASE_DIR/target/release/vrift"
export PATH="$BASE_DIR/target/release:$PATH"

# Path normalization for macOS (/tmp -> /private/tmp)
PROJECT_A=$(realpath "$TEST_ROOT/project_a")
PROJECT_B=$(realpath "$TEST_ROOT/project_b")

# 1. Setup Projects
echo "Hello project A" > "$PROJECT_A/file_a.txt"
echo "Hello project B" > "$PROJECT_B/file_b.txt"

# 2. Ingest
echo "📦 Ingesting Project A..."
"$VR_BIN" ingest "$PROJECT_A" --output "$PROJECT_A/vrift.manifest" --prefix ""

echo "📦 Ingesting Project B..."
"$VR_BIN" ingest "$PROJECT_B" --output "$PROJECT_B/vrift.manifest" --prefix ""

# 3. Start Daemon
echo "🔍 Checking Daemon State..."
# Kill any existing daemon
pkill vriftd || true
sleep 1
# Start daemon in background
export RUST_LOG=info
vriftd start > "$TEST_ROOT/vriftd.log" 2>&1 &
sleep 2

# Register A
echo "🔗 Registering Project A..."
"$VR_BIN" daemon status --directory "$PROJECT_A" || { cat "$TEST_ROOT/vriftd.log"; exit 1; }

# Check if project_a manifest exists in .vrift
if [ ! -f "$PROJECT_A/.vrift/daemon_manifest.lmdb/data.mdb" ]; then
    echo -e "${RED}FAILED: Project A manifest not found in .vrift${NC}"
    ls -R "$PROJECT_A/.vrift"
    exit 1
fi

# Register B
echo "🔗 Registering Project B..."
"$VR_BIN" daemon status --directory "$PROJECT_B" || { cat "$TEST_ROOT/vriftd.log"; exit 1; }

if [ ! -f "$PROJECT_B/.vrift/daemon_manifest.lmdb/data.mdb" ]; then
    echo -e "${RED}FAILED: Project B manifest not found in .vrift${NC}"
    exit 1
fi

# 4. Verify VFS Isolation (Shim)
echo "🛡️ Verifying Shim Isolation..."

# Compile test_stat
gcc "$BASE_DIR/test_stat.c" -o "$TEST_ROOT/test_stat"
codesign --force --sign - "$TEST_ROOT/test_stat"

cd "$PROJECT_A"
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_MANIFEST="$PROJECT_A/vrift.manifest"
export VRIFT_DEBUG=1
export VRIFT_DISABLE_MMAP=1

echo "   Testing Project A view..."
"$VR_BIN" run --manifest "$PROJECT_A/vrift.manifest" "$TEST_ROOT/test_stat" 2>&1 | tee "$TEST_ROOT/test_a.log"
grep "SUCCESS: stat(\"/vrift/file_a.txt\") worked!" "$TEST_ROOT/test_a.log" || { echo "Project A verification failed"; cat "$TEST_ROOT/test_a.log"; cat "$TEST_ROOT/vriftd.log"; exit 1; }

echo "   Testing Project B view (should NOT see A's file if we change manifest)..."
cd "$PROJECT_B"
export VRIFT_MANIFEST="$PROJECT_B/vrift.manifest"
"$VR_BIN" run --manifest "$PROJECT_B/vrift.manifest" "$TEST_ROOT/test_stat" 2>&1 | tee "$TEST_ROOT/test_b.log"
grep "SUCCESS: stat(\"/vrift/file_b.txt\") worked!" "$TEST_ROOT/test_b.log" || { echo "Project B verification failed"; cat "$TEST_ROOT/test_b.log"; cat "$TEST_ROOT/vriftd.log"; exit 1; }

echo -e "${GREEN}✅ Phase 6 Isolation Test PASSED${NC}"
pkill vriftd || true
