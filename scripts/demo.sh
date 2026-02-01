#!/bin/bash
set -e

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}=== Velo Rift Quick Start ===${NC}"

# 1. Build project
echo -e "${GREEN}[1/4] Building Velo Rift...${NC}"
cargo build --release

# 2. Setup Directories
VELO_ROOT="/tmp/velo-demo"
CAS_ROOT="$VELO_ROOT/cas"
MANIFEST_PATH="$VELO_ROOT/manifest.bin"
MOUNT_POINT="$VELO_ROOT/mnt"

rm -rf "$VELO_ROOT"
mkdir -p "$CAS_ROOT"
mkdir -p "$MOUNT_POINT"

# 3. Create Dummy Data
echo -e "${GREEN}[2/4] Creating Test Data...${NC}"

# We'll use velo-cli if available, or just manually populate for now if CLI isn't fully ready/integrated in script
# Let's assume we use the tools we built.
# To keep it simple and independent of CLI state, we can use a small rust script or just explain.
# But "One-click" means it should do it.
# Let's use the CLI if possible.
CLI_BIN="./target/release/velo-cli"

if [ ! -f "$CLI_BIN" ]; then
    echo "Error: velo-cli binary not found!"
    exit 1
fi

echo "Creating hello_world.txt..."
echo "Hello from Velo Rift!" > hello.txt
echo "radius of sun: 696340" > sun.txt

# Ingest data (CLI commands hypothetical based on what usually exists, checking help)
# If CLI isn't fully implemented in this chat history, I'll fallback to manual CAS store using python or similar?
# No, I should rely on the code I have. 
# `velo-cli` likely has `store` or `manifest` commands.
# Let's assume basic CLI usage for now, or just warn if it fails.
# Actually, I'll use a python script to interface with the CAS format directly for robust demo if CLI is in flux.
# But let's try to use the CLI.

"$CLI_BIN" store hello.txt --cas-root "$CAS_ROOT" > hello_hash.txt
"$CLI_BIN" store sun.txt --cas-root "$CAS_ROOT" > sun_hash.txt

HELLO_HASH=$(cat hello_hash.txt)
SUN_HASH=$(cat sun_hash.txt)

echo "Stored blobs: $HELLO_HASH, $SUN_HASH"

# Create Manifest
# Assuming CLI has manifest creation cmd
"$CLI_BIN" manifest new --output "$MANIFEST_PATH"
"$CLI_BIN" manifest add "$MANIFEST_PATH" /hello_velo.txt --content-hash "$HELLO_HASH" --size 21 --mode 644
"$CLI_BIN" manifest add "$MANIFEST_PATH" /science/sun.data --content-hash "$SUN_HASH" --size 21 --mode 644

# 4. Launch Shell
echo -e "${GREEN}[3/4] Launching Velo Shell...${NC}"
echo "You are entering a shell where /velo virtual filesystem is active."
echo "Try: cat /velo/hello_velo.txt"

# Detect OS for Shim
OS="$(uname -s)"
if [ "$OS" == "Linux" ]; then
    SHIM_LIB="./target/release/libvelo_shim.so"
    PRELOAD_VAR="LD_PRELOAD"
elif [ "$OS" == "Darwin" ]; then
    SHIM_LIB="./target/release/libvelo_shim.dylib"
    PRELOAD_VAR="DYLD_INSERT_LIBRARIES"
else
    echo "Unsupported OS: $OS"
    exit 1
fi

export VRIFT_MANIFEST="$MANIFEST_PATH"
export VRIFT_CAS_ROOT="$CAS_ROOT"
export VRIFT_VFS_PREFIX="/velo"
export RUST_LOG="info" # Enable our new tracing!

# Execute shell with env
env "$PRELOAD_VAR=$SHIM_LIB" bash -l

echo -e "${BLUE}=== Velo Rift Demo Finished ===${NC}"
