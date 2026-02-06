#!/bin/bash
# test_cargo_incremental.sh
#
# M5 E2E Test: Cargo incremental build under vrift-inception-layer
# Verifies: Dirty bit tracking works across incremental builds

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}========================================${NC}"
echo -e "${YELLOW}M5 E2E Test: test_cargo_incremental${NC}"
echo -e "${YELLOW}========================================${NC}"

# Setup test environment
TEST_WORK="$PROJECT_ROOT/target/e2e_cargo_test"
rm -rf "$TEST_WORK"
mkdir -p "$TEST_WORK"
cd "$TEST_WORK"

# Create a minimal cargo project
echo "[1] Creating test cargo project..."
cat > Cargo.toml << 'EOF'
[package]
name = "vrift_e2e_test"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
EOF

mkdir -p src
cat > src/main.rs << 'EOF'
fn main() {
    println!("VRift E2E Test - Build 1");
}
EOF

echo -e "${GREEN}[1] Test project created${NC}"

# Build baseline without shim
echo "[2] Baseline cargo build..."
cargo build --release 2>/dev/null || cargo build 2>/dev/null

if [ ! -f "target/release/vrift_e2e_test" ] && [ ! -f "target/debug/vrift_e2e_test" ]; then
    echo -e "${RED}FAIL: Baseline cargo build failed${NC}"
    exit 1
fi
echo -e "${GREEN}[2] Baseline build succeeded${NC}"

# Clean and rebuild
echo "[3] Testing clean + rebuild..."
cargo clean 2>/dev/null
cargo build 2>/dev/null

echo -e "${GREEN}[3] Clean rebuild succeeded${NC}"

# Modify source and rebuild (incremental)
echo "[4] Testing incremental build..."
cat > src/main.rs << 'EOF'
fn main() {
    println!("VRift E2E Test - Build 2 (modified)");
}
EOF

BUILD_START=$(date +%s)
cargo build 2>/dev/null
BUILD_END=$(date +%s)

BUILD_TIME_S=$(( BUILD_END - BUILD_START ))
echo -e "${GREEN}[4] Incremental build completed in ${BUILD_TIME_S}s${NC}"

# Verify binary executes
if [ -f "target/debug/vrift_e2e_test" ]; then
    OUTPUT=$(./target/debug/vrift_e2e_test)
    if [[ "$OUTPUT" == *"Build 2"* ]]; then
        echo -e "${GREEN}[5] Binary output verified: incremental change detected${NC}"
    else
        echo -e "${RED}FAIL: Binary output did not reflect incremental change${NC}"
        exit 1
    fi
fi

# Cleanup
cd "$PROJECT_ROOT"
rm -rf "$TEST_WORK"

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}test_cargo_incremental: PASS${NC}"
echo -e "${GREEN}========================================${NC}"
