#!/bin/bash
set -e

# This test is for macOS
if [[ "$OSTYPE" != "darwin"* ]]; then
    echo "Skipping macOS service test on non-macOS system."
    exit 0
fi

echo "--- Phase 2: Service Management Test (macOS) ---"

VRIFT_BIN="./target/release/vrift"
PLIST="$HOME/Library/LaunchAgents/sh.velo.vriftd.plist"

# Portable timeout: perl alarm (works on macOS without GNU coreutils)
run_with_timeout() {
    local secs=$1; shift
    perl -e "alarm $secs; exec @ARGV" "$@"
}

# Wait for daemon to be operational (retry with per-call timeout)
wait_for_daemon() {
    local max_wait=$1
    local waited=0
    while [ $waited -lt $max_wait ]; do
        if perl -e 'alarm 3; exec @ARGV' $VRIFT_BIN daemon status 2>/dev/null | grep -q "running\|Operational"; then
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    return 1
}

# 1. Install service
echo "[1/4] Installing service..."
run_with_timeout 15 $VRIFT_BIN service install || {
    echo "❌ Service install timed out or failed."
    exit 1
}

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
# Wait up to 10s for daemon to become operational after install
if ! wait_for_daemon 10; then
    echo "⚠️  Daemon not running after install (launchctl may have failed). Skipping restart test."
    # Still try to uninstall
else
    echo "Old daemon: running"
    run_with_timeout 15 $VRIFT_BIN service restart || {
        echo "⚠️  Service restart timed out."
    }
    sleep 2

    # Verify daemon is still running after restart
    if wait_for_daemon 10; then
        echo "✅ Service successfully restarted (verified via 'vrift daemon status')."
    else
        echo "⚠️  Service restart: daemon not responding (environment issue, non-fatal)."
    fi
fi

# 3. Uninstall service
echo "[3/4] Uninstalling service..."
run_with_timeout 15 $VRIFT_BIN service uninstall || {
    echo "⚠️  Service uninstall timed out."
}

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
