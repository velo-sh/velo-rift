#!/bin/bash

# This script verifies if the Daemon can distinguish between different users
# making IPC calls, or if it blindly obeys any request (RFC-0043 §8 Alignment).

echo "--- User Isolation Functional Verification ---"

SOCKET_PATH="/tmp/vrift.sock"
DAEMON_LOG="/tmp/vriftd_isolation.log"

# 1. Start Daemon
pkill vriftd || true
export VR_THE_SOURCE="/tmp/vrift_source_isolation"
mkdir -p "$VR_THE_SOURCE"
(
    unset DYLD_INSERT_LIBRARIES
    unset DYLD_FORCE_FLAT_NAMESPACE
    ./target/debug/vriftd start > "$DAEMON_LOG" 2>&1 &
    echo $! > /tmp/vriftd_isolation.pid
)
sleep 1

# 2. Check if Daemon has any logic to identify the caller
# We'll use the trigger_exploit example but look for identity logs
echo "[+] Sending IPC request..."
./target/debug/examples/trigger_exploit > /dev/null 2>&1

# 3. Analyze logs for user identification
echo "[+] Analyzing Daemon logs for user identification..."
# We expect to find NO logs about UIDs or GIDs because the code doesn't have it.
if grep -Ei "uid|gid|user|peer" "$DAEMON_LOG"; then
    echo "[SUCCESS] Daemon is attempting to identify the peer."
else
    echo "[FAIL] Daemon blindly processes requests without identifying the peer UID/GID."
    echo "       (This violates the multi-user isolation requirement of RFC-0043 §8)"
fi

# 4. Check socket permissions
echo "[+] Checking socket permissions..."
ls -l "$SOCKET_PATH"
PERMS=$(ls -l "$SOCKET_PATH" | awk '{print $1}')
if [[ "$PERMS" == "srwxrwxrwx" ]]; then
    echo "[WARNING] Socket is 777. Any user on the system can talk to the daemon."
else
    echo "[INFO] Socket permissions: $PERMS"
fi

pkill vriftd
rm -rf "$VR_THE_SOURCE" "$DAEMON_LOG"
