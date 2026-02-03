#!/bin/bash
set -e

# This test is for macOS
if [[ "$OSTYPE" != "darwin"* ]]; then
    echo "Skipping macOS service test on non-macOS system."
    exit 0
fi

echo "--- Phase 2: Service Management Test (macOS) ---"

VRIFT_BIN="/Users/antigravity/rust_source/vrift_qa/target/release/vrift"
PLIST="$HOME/Library/LaunchAgents/sh.velo.vriftd.plist"

# 1. Install service
echo "[1/4] Installing service..."
$VRIFT_BIN service install

if [ -f "$PLIST" ]; then
    echo "✅ Launchd plist created at $PLIST."
else
    echo "❌ Launchd plist NOT found."
    exit 1
fi

if launchctl list | grep -q "sh.velo.vriftd"; then
    echo "✅ Service registered in launchctl."
else
    echo "❌ Service NOT found in launchctl list."
    exit 1
fi

# 2. Restart service
echo "[2/4] Testing service restart..."
OLD_PID=$(pgrep vriftd || echo "")
echo "Old PID: $OLD_PID"

$VRIFT_BIN service restart
sleep 2

NEW_PID=$(pgrep vriftd || echo "")
echo "New PID: $NEW_PID"

if [ "$OLD_PID" != "$NEW_PID" ] && [ -n "$NEW_PID" ]; then
    echo "✅ Service successfully restarted with new PID."
else
    echo "❌ Service restart failed or PID didn't change."
    exit 1
fi

# 3. Uninstall service
echo "[3/4] Uninstalling service..."
$VRIFT_BIN service uninstall

if [ ! -f "$PLIST" ]; then
    echo "✅ Launchd plist removed."
else
    echo "❌ Launchd plist still exists."
    exit 1
fi

if ! launchctl list | grep -q "sh.velo.vriftd"; then
    echo "✅ Service removed from launchctl."
else
    echo "❌ Service still present in launchctl list."
    exit 1
fi

echo "--- Test PASSED ---"
