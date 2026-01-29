#!/bin/bash
set -e

MODE="host"

if [[ "$1" == "--docker" ]]; then
    MODE="docker"
fi

echo "=== Velo Rift Local CI ($MODE) ==="

if [[ "$MODE" == "docker" ]]; then
    # Docker Mode
    echo "[*] Building Base Image (Layer Caching Enabled)..."
    docker build -t velo-ci-base -f Dockerfile.base .

    echo "[*] Building CI Image..."
    docker build -t velo-ci-e2e -f Dockerfile.ci .

    echo "[*] Running Test Suite in Docker..."
    docker run --rm --privileged velo-ci-e2e
else
    # Host Mode (macOS/Linux)
    echo "[*] Running Test Suite on Host..."
    ./test.sh
fi

echo "=== Local CI Passed ==="
