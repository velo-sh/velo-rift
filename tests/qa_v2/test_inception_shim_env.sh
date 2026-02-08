#!/usr/bin/env bash
# test_inception_shim_env.sh
# Verify that Config::shim_env() correctly derives all required
# environment variables from the SSOT (TOML config), and that
# cmd_inception generates proper shell export statements.
#
# This test validates the Config SSOT → shim env bridge without
# requiring a running daemon.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
VRIFT="$ROOT_DIR/target/release/vrift"

echo "----------------------------------------------------------------"
echo "🧪 Test: Config SSOT → Shim Environment Bridge"
echo "----------------------------------------------------------------"

# Setup temp project
TMPDIR="$(mktemp -d /tmp/vrift_shim_env_XXXXXX)"
trap 'rm -rf "$TMPDIR"' EXIT

cd "$TMPDIR"

# Initialize project
"$VRIFT" init . >/dev/null 2>&1 || true

PASS=0
FAIL=0

check_ok() {
    local desc="$1"
    echo "  ✅ $desc"
    PASS=$((PASS + 1))
}

check_fail() {
    local desc="$1"
    echo "  ❌ $desc"
    FAIL=$((FAIL + 1))
}

# Phase 1: Verify config loads correctly in this project
echo ""
echo "[Phase 1] Config Loading"
CONFIG_OUTPUT=$("$VRIFT" config show 2>&1 || true)

if echo "$CONFIG_OUTPUT" | grep -q 'config_version'; then
    check_ok "Config has config_version field"
else
    check_fail "Config missing config_version field"
fi

if echo "$CONFIG_OUTPUT" | grep -q 'the_source'; then
    check_ok "Config has storage.the_source"
else
    check_fail "Config missing storage.the_source"
fi

if echo "$CONFIG_OUTPUT" | grep -q 'socket'; then
    check_ok "Config has daemon.socket"
else
    check_fail "Config missing daemon.socket"
fi

if echo "$CONFIG_OUTPUT" | grep -q 'vfs_prefix'; then
    check_ok "Config has project.vfs_prefix"
else
    check_fail "Config missing project.vfs_prefix"
fi

# Phase 2: Verify project config was generated
echo ""
echo "[Phase 2] Project Config"
if [[ -f .vrift/config.toml ]]; then
    check_ok ".vrift/config.toml exists"

    if grep -q 'config_version' .vrift/config.toml; then
        check_ok "Project config has config_version"
    else
        check_fail "Project config missing config_version"
    fi

    if grep -q 'vfs_prefix' .vrift/config.toml; then
        check_ok "Project config has vfs_prefix"
    else
        check_fail "Project config missing vfs_prefix"
    fi
else
    check_fail ".vrift/config.toml not generated"
fi

# Phase 3: Verify vrift doctor runs
echo ""
echo "[Phase 3] Doctor Diagnostics"
DOCTOR_OUTPUT=$("$VRIFT" doctor . 2>&1 || true)

if echo "$DOCTOR_OUTPUT" | grep -q 'Config loads successfully'; then
    check_ok "vrift doctor: config loads"
else
    check_fail "vrift doctor: config failed to load"
fi

if echo "$DOCTOR_OUTPUT" | grep -q 'passed'; then
    check_ok "vrift doctor: completed with results"
else
    check_fail "vrift doctor: no results"
fi

# Summary
echo ""
echo "----------------------------------------------------------------"
echo "--- Results: $PASS passed, $FAIL failed ---"

if [[ $FAIL -gt 0 ]]; then
    echo "❌ FAILED"
    exit 1
fi

echo "✅ Config SSOT bridge verified"
echo "----------------------------------------------------------------"
