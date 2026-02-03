#!/bin/bash
set -e

echo "--- Phase 2: Daemon Auto-start Test ---"

# 1. Cleanup
echo "[1/4] Cleaning up environment..."
pkill vriftd || true
rm -f /tmp/vrift.sock

# 2. Run CLI command that requires daemon
echo "[2/4] Running 'vrift daemon status'..."
RUST_LOG=info ./target/release/vrift daemon status > autostart.log 2>&1
cat autostart.log

# 3. Verify auto-start triggered
if grep -q "Daemon not running. Attempting to start..." autostart.log; then
    echo "✅ CLI detected daemon was missing and attempted start."
else
    echo "❌ CLI failed to detect mission daemon or log message changed."
    exit 1
fi

# 4. Verify daemon is actually running
sleep 1
if [ -S /tmp/vrift.sock ]; then
    echo "✅ /tmp/vrift.sock exists."
else
    echo "❌ /tmp/vrift.sock NOT found."
    exit 1
fi

if pgrep vriftd > /dev/null; then
    echo "✅ vriftd process is running (PID: $(pgrep vriftd))."
else
    echo "❌ vriftd process NOT found."
    exit 1
fi

echo "--- Test PASSED ---"
rm autostart.log
