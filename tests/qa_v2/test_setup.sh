#!/bin/bash
# ==============================================================================
# Velo Rift Test Setup Helper
# ==============================================================================
# Source this file in test scripts to get standardized test isolation.
#
# Usage:
#   source "$(dirname "${BASH_SOURCE[0]}")/test_setup.sh"
#
# Provides:
#   - Unique VRIFT_SOCKET_PATH per test run
#   - VR_THE_SOURCE set to isolated CAS root
#   - VRIFT_PROJECT_ROOT set to test workspace
#   - Standard cleanup trap
#   - Helper functions for daemon management
# ==============================================================================

set -euo pipefail

# ============================================================================
# Configuration (auto-generated, unique per test run)
# ============================================================================
SCRIPT_NAME="$(basename "${BASH_SOURCE[1]:-unknown}")"
TEST_ID="${SCRIPT_NAME%.sh}_$$"
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Standard paths
TEST_WORKSPACE="/tmp/vrift_test_${TEST_ID}"
VR_THE_SOURCE="${TEST_WORKSPACE}/.cas"
VRIFT_SOCKET_PATH="${TEST_WORKSPACE}/vrift.sock"

# Binaries
VRIFT_CLI="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"
SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
[ ! -f "$SHIM_LIB" ] && SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.so"

# Export for child processes
export VR_THE_SOURCE
export VRIFT_SOCKET_PATH
export VRIFT_PROJECT_ROOT="${TEST_WORKSPACE}"

# ============================================================================
# Platform Detection
# ============================================================================
OS="$(uname -s)"
if [ "$OS" = "Darwin" ]; then
    VFS_ENV_BASE="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1"
else
    VFS_ENV_BASE="LD_PRELOAD=$SHIM_LIB"
fi

# ============================================================================
# Daemon Management
# ============================================================================
VRIFTD_PID=""

start_daemon() {
    local log_level="${1:-info}"
    
    echo "   Starting vriftd (socket: $VRIFT_SOCKET_PATH)..."
    VRIFT_LOG="$log_level" VR_THE_SOURCE="$VR_THE_SOURCE" \
        "$VRIFTD_BIN" start </dev/null > "${TEST_WORKSPACE}/vriftd.log" 2>&1 &
    VRIFTD_PID=$!
    
    # Wait for socket with timeout
    local waited=0
    while [ ! -S "$VRIFT_SOCKET_PATH" ] && [ $waited -lt 10 ]; do
        sleep 0.5
        waited=$((waited + 1))
    done
    
    if [ -S "$VRIFT_SOCKET_PATH" ]; then
        echo "   ✓ Daemon running (PID: $VRIFTD_PID)"
        return 0
    else
        echo "   ✗ Daemon failed to start"
        [ -f "${TEST_WORKSPACE}/vriftd.log" ] && cat "${TEST_WORKSPACE}/vriftd.log"
        return 1
    fi
}

stop_daemon() {
    if [ -n "$VRIFTD_PID" ]; then
        kill -TERM "$VRIFTD_PID" 2>/dev/null || true
        wait "$VRIFTD_PID" 2>/dev/null || true
        VRIFTD_PID=""
    fi
    
    # Kill any daemon associated with our socket
    if [ -S "$VRIFT_SOCKET_PATH" ]; then
        rm -f "$VRIFT_SOCKET_PATH"
    fi
}

# ============================================================================
# Cleanup
# ============================================================================
test_cleanup() {
    local exit_code=$?
    
    stop_daemon
    
    # Kill any stray processes with our socket path
    pkill -f "vriftd.*${TEST_WORKSPACE}" 2>/dev/null || true
    
    # Remove immutable flags (macOS) and cleanup
    if [ -d "$TEST_WORKSPACE" ]; then
        chflags -R nouchg "$TEST_WORKSPACE" 2>/dev/null || true
        rm -rf "$TEST_WORKSPACE"
    fi
    
    return $exit_code
}
trap test_cleanup EXIT

# ============================================================================
# Setup
# ============================================================================
setup_test_workspace() {
    # Cleanup any previous run
    if [ -d "$TEST_WORKSPACE" ]; then
        chflags -R nouchg "$TEST_WORKSPACE" 2>/dev/null || true
        rm -rf "$TEST_WORKSPACE"
    fi
    
    mkdir -p "${TEST_WORKSPACE}/src"
    mkdir -p "${VR_THE_SOURCE}"
    
    cd "$TEST_WORKSPACE"
    
    echo "   Test ID:      $TEST_ID"
    echo "   Workspace:    $TEST_WORKSPACE"
    echo "   CAS Root:     $VR_THE_SOURCE"
    echo "   Socket Path:  $VRIFT_SOCKET_PATH"
}

# ============================================================================
# VFS Environment Helper
# ============================================================================
# Usage: run_with_shim <command> [args...]
run_with_shim() {
    env $VFS_ENV_BASE \
        VRIFT_PROJECT_ROOT="$TEST_WORKSPACE" \
        VRIFT_VFS_PREFIX="$TEST_WORKSPACE" \
        VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" \
        VR_THE_SOURCE="$VR_THE_SOURCE" \
        VRIFT_DEBUG=1 \
        "$@"
}

# ============================================================================
# Auto-setup if sourced
# ============================================================================
echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  Velo Rift Test: $SCRIPT_NAME"
echo "╚══════════════════════════════════════════════════════════════════════╝"

setup_test_workspace
