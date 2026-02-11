#!/bin/bash
set -e

# Setup environment
export VRIFT_PROJECT_ROOT="/Users/antigravity/rust_source/velo"
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_SOCKET_PATH="/tmp/vrift.sock"
export VR_THE_SOURCE="/Users/antigravity/rust_source/velo-rift/.vrift/the_source"
export VRIFT_VDIR_MMAP="/Users/antigravity/rust_source/velo-rift/.vrift/vdir.mmap"
export VRIFT_DEBUG=1
export VRIFT_INCEPTION=1

# Use compiled inception layer
export DYLD_INSERT_LIBRARIES="/Users/antigravity/rust_source/velo-rift/target/debug/libvrift_inception_layer.dylib"
export DYLD_FORCE_FLAT_NAMESPACE=1

cd "$VRIFT_PROJECT_ROOT"

# Clean build
echo "Starting clean build in inception mode..."
cargo clean
cargo build -j1
echo "Build successful!"
