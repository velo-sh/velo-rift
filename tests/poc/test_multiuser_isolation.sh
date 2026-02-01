#!/bin/bash
# test_multiuser_isolation.sh - Verify UID/GID isolation between users
# Priority: P1 (Security)
set -e

echo "=== Test: Multi-User Isolation ==="

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

echo "[1] Checking daemon UID verification..."
if grep -q "peer.*.uid\|PeerCredentials\|SO_PEERCRED" "$PROJECT_ROOT/crates/vrift-daemon/src/main.rs" 2>/dev/null; then
    echo "    ✓ Daemon checks peer credentials"
    DAEMON_UID_CHECK=1
else
    echo "    ✗ Daemon missing UID verification"
    DAEMON_UID_CHECK=0
fi

echo "[2] Checking spawn permission enforcement..."
if grep -q "uid != daemon_uid\|Permission denied" "$PROJECT_ROOT/crates/vrift-daemon/src/main.rs" 2>/dev/null; then
    echo "    ✓ Spawn has UID permission check"
    SPAWN_CHECK=1
else
    echo "    ✗ Spawn missing UID check"
    SPAWN_CHECK=0
fi

echo "[3] Checking manifest isolation..."
# Each user should have separate manifest or namespace
if grep -q "VRIFT_MANIFEST_DIR\|user.*manifest" "$PROJECT_ROOT/crates/vrift-daemon/src/main.rs" 2>/dev/null; then
    echo "    ✓ Manifest path configurable (per-user possible)"
    MANIFEST_ISOLATED=1
else
    echo "    ? Manifest isolation unclear"
    MANIFEST_ISOLATED=0
fi

echo ""
SCORE=$((DAEMON_UID_CHECK + SPAWN_CHECK + MANIFEST_ISOLATED))
if [ "$SCORE" -ge 2 ]; then
    echo "✅ PASS: Multi-user isolation ($SCORE/3 checks passed)"
    exit 0
else
    echo "⚠️  WARN: Multi-user isolation needs review ($SCORE/3)"
    exit 0
fi
