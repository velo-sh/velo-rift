#!/bin/bash
# VRift Native Linux Verification Script
# This script runs the verified functional tiers (1 & 2) on the native Linux host.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "=== VRift Native Linux Verification ==="
echo "Host: $(uname -a)"

# 1. Environment Checks
echo "[*] Checking environment..."
./scripts/v-ci --local --setup

# 2. Build with FUSE (if possible)
echo "[*] Building VRift release..."
./scripts/v-ci --local --tier 0 --rust-args --release

# 3. Run Tier 1 (Core VFS & ABI)
echo "[*] Running Tier 1 (Core Functional)..."
./scripts/v-ci --local --tier 1

# 4. Run Tier 2 (E2E & Historical Bugs)
echo "[*] Running Tier 2 (E2E Regression)..."
./scripts/v-ci --local --tier 2

echo "=== Native Linux Verification Complete ==="
echo "✅ All verified tiers passed natively."
