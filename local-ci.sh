#!/bin/bash
# =============================================================================
# VRift Local CI (Proxy)
# =============================================================================
# This script is a compatibility wrapper for scripts/v-ci.
# Please consider using scripts/v-ci directly.
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Map old flags to new system
ARGS=()
for arg in "$@"; do
    case "$arg" in
        --docker) ARGS+=("--docker") ;;
        --rebuild-base) ARGS+=("--build") ;;
        *) ARGS+=("$arg") ;;
    esac
done

exec "$SCRIPT_DIR/scripts/v-ci" "${ARGS[@]}"
