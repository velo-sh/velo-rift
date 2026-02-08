#!/bin/bash
# ==============================================================================
# Velo Rift Test Setup Helper (SSOT)
# ==============================================================================
# Source this file in test scripts to get standardized test isolation.
#
# Usage:
#   source "$(dirname "${BASH_SOURCE[0]}")/test_setup.sh"
#
# Provides (all exported for child processes):
#   - PROJECT_ROOT: Path to velo-rift repo root
#   - TEST_WORKSPACE: Unique temp directory for this test
#   - VR_THE_SOURCE: Isolated CAS root
#   - VRIFT_SOCKET_PATH: Unique socket path per test
#   - VRIFT_PROJECT_ROOT: Same as TEST_WORKSPACE
#   - VRIFT_CLI, VRIFTD_BIN, SHIM_LIB: Standard binary paths
#   - VFS_ENV_BASE: Platform-specific DYLD/LD env for shim injection
#
# Overridable variables (set BEFORE sourcing):
#   - TEST_WORKSPACE_BASE: Override /tmp/vrift_test_*
#   - SKIP_AUTO_SETUP: Set to 1 to skip auto setup
#
# Helper functions:
#   - setup_test_workspace: Initialize test workspace
#   - start_daemon [log_level]: Start daemon with optional log level
#   - stop_daemon: Stop daemon cleanly
#   - run_with_shim <cmd>: Run command with shim injected
#   - test_cleanup: Called automatically on EXIT
# ==============================================================================

set -euo pipefail

# Source shared test harness (logging, counters, summary)
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/test_harness.sh"

# ============================================================================
# Core Configuration (always set)
# ============================================================================
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_NAME="$(basename "${BASH_SOURCE[1]:-unknown}")"
TEST_ID="${SCRIPT_NAME%.sh}_$$"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Standard paths
TEST_WORKSPACE="${TEST_WORKSPACE_BASE:-/tmp/vrift_test_${TEST_ID}}"
VR_THE_SOURCE="${TEST_WORKSPACE}/.cas"
VRIFT_SOCKET_PATH="${TEST_WORKSPACE}/vrift.sock"

# Binaries
VRIFT_CLI="${PROJECT_ROOT}/target/release/vrift"
VRIFTD_BIN="${PROJECT_ROOT}/target/release/vriftd"
VDIRD_BIN="${PROJECT_ROOT}/target/release/vrift-vdird"
SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
[ ! -f "$SHIM_LIB" ] && SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.so"

# Fallback to debug builds if release not found
[ ! -f "$VRIFT_CLI" ] && VRIFT_CLI="${PROJECT_ROOT}/target/debug/vrift"
[ ! -f "$VRIFTD_BIN" ] && VRIFTD_BIN="${PROJECT_ROOT}/target/debug/vriftd"
[ ! -f "$VDIRD_BIN" ] && VDIRD_BIN="${PROJECT_ROOT}/target/debug/vrift-vdird"
[ ! -f "$SHIM_LIB" ] && SHIM_LIB="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
[ ! -f "$SHIM_LIB" ] && SHIM_LIB="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.so"

# Export for child processes
export PROJECT_ROOT
export TEST_WORKSPACE
export VR_THE_SOURCE
export VRIFT_SOCKET_PATH
export VRIFT_PROJECT_ROOT="${TEST_WORKSPACE}"
export VRIFT_CLI
export VRIFTD_BIN
export VDIRD_BIN
export SHIM_LIB

# ============================================================================
# Platform Detection
# ============================================================================
OS="$(uname -s)"
if [ "$OS" = "Darwin" ]; then
    VFS_ENV_BASE="DYLD_INSERT_LIBRARIES=$SHIM_LIB DYLD_FORCE_FLAT_NAMESPACE=1"
else
    VFS_ENV_BASE="LD_PRELOAD=$SHIM_LIB"
fi
export VFS_ENV_BASE

# ============================================================================
# Daemon Management
# ============================================================================
VRIFTD_PID=""

# Ensure vdir_d symlink exists next to vriftd (vDird subprocess model)
ensure_vdird_symlink() {
    local vriftd_dir
    vriftd_dir="$(dirname "$VRIFTD_BIN")"
    if [ -f "$VDIRD_BIN" ] && [ ! -e "${vriftd_dir}/vdir_d" ]; then
        ln -sf "$(basename "$VDIRD_BIN")" "${vriftd_dir}/vdir_d"
    fi
}

start_daemon() {
    local log_level="${1:-info}"
    
    ensure_vdird_symlink
    echo "   Starting vriftd (socket: $VRIFT_SOCKET_PATH)..."
    VRIFT_LOG="$log_level" VR_THE_SOURCE="$VR_THE_SOURCE" \
        VRIFT_SOCKET_PATH="$VRIFT_SOCKET_PATH" \
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
    if [ -n "${VRIFTD_PID:-}" ]; then
        kill -TERM "$VRIFTD_PID" 2>/dev/null || true
        wait "$VRIFTD_PID" 2>/dev/null || true
        VRIFTD_PID=""
    fi
    
    # Kill any daemon associated with our socket
    if [ -S "${VRIFT_SOCKET_PATH:-}" ]; then
        rm -f "$VRIFT_SOCKET_PATH"
    fi
}

# ============================================================================
# Cleanup (trap on EXIT)
# ============================================================================
test_cleanup() {
    local exit_code=$?
    
    stop_daemon
    
    # Kill any stray processes with our socket path
    pkill -f "vriftd.*${TEST_WORKSPACE:-nonexistent}" 2>/dev/null || true
    
    # Remove immutable flags (macOS) and cleanup
    if [ -d "${TEST_WORKSPACE:-}" ]; then
        chflags -R nouchg "$TEST_WORKSPACE" 2>/dev/null || true
        rm -rf "$TEST_WORKSPACE"
    fi
    
    return $exit_code
}
trap test_cleanup EXIT

# ============================================================================
# Workspace Setup
# ============================================================================
setup_test_workspace() {
    # Cleanup any previous run
    if [ -d "$TEST_WORKSPACE" ]; then
        chflags -R nouchg "$TEST_WORKSPACE" 2>/dev/null || true
        rm -rf "$TEST_WORKSPACE"
    fi
    
    mkdir -p "${TEST_WORKSPACE}/src"
    mkdir -p "${VR_THE_SOURCE}"
    
    # Ensure vdir_d symlink for vDird subprocess model
    ensure_vdird_symlink
    
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
# Prerequisite Checks
# ============================================================================
check_prerequisites() {
    local missing=0
    
    if [ ! -f "$VRIFT_CLI" ]; then
        echo "❌ vrift CLI not found: $VRIFT_CLI"
        echo "   Run: cargo build --release"
        missing=1
    fi
    
    if [ ! -f "$VRIFTD_BIN" ]; then
        echo "❌ vriftd not found: $VRIFTD_BIN"
        echo "   Run: cargo build --release"
        missing=1
    fi
    
    if [ ! -f "$VDIRD_BIN" ]; then
        echo "❌ vrift-vdird not found: $VDIRD_BIN"
        echo "   Run: cargo build --release"
        missing=1
    fi
    
    if [ ! -f "$SHIM_LIB" ]; then
        echo "❌ Shim library not found: $SHIM_LIB"
        echo "   Run: cargo build --release"
        missing=1
    fi
    
    return $missing
}

# ============================================================================
# Auto-setup (unless SKIP_AUTO_SETUP=1)
# ============================================================================
if [ "${SKIP_AUTO_SETUP:-0}" != "1" ]; then
    echo ""
    echo "╔══════════════════════════════════════════════════════════════════════╗"
    echo "║  Velo Rift Test: $SCRIPT_NAME"
    echo "╚══════════════════════════════════════════════════════════════════════╝"
    
    setup_test_workspace
fi
