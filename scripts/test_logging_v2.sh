#!/bin/bash
set -e

# Auto-detect target directory (prefer release)
PROJECT_ROOT=$(pwd)
if [ -d "$PROJECT_ROOT/target/release" ]; then
    TARGET_DIR="release"
else
    TARGET_DIR="debug"
fi

OS=$(uname -s)
if [ "$OS" == "Darwin" ]; then
    EXT="dylib"
else
    EXT="so"
fi

SHIM_PATH="$PROJECT_ROOT/target/$TARGET_DIR/libvrift_shim.$EXT"

# Ensure work dir exists
mkdir -p test_logging_work

# Build the test tool
gcc -o test_logging_work/simple_open scripts/simple_open.c

# 1. Test Flight Recorder via SIGUSR1
echo "--- Testing Flight Recorder Dump ---"
VRIFT_ENABLE_SIGNAL_HANDLERS=1 VRIFT_DEBUG=1 DYLD_INSERT_LIBRARIES=$SHIM_PATH ./test_logging_work/simple_open 60 > test_logging_work/full_log.txt 2>&1 &
PID=$!
sleep 2
echo "Sending SIGUSR1 to $PID"
kill -USR1 $PID
sleep 2
kill $PID || true
wait $PID 2>/dev/null || true

echo "--- Check if Logs contains Dump header ---"
grep "Flight Recorder Dump" test_logging_work/full_log.txt || (echo "Dump header not found"; cat test_logging_work/full_log.txt; exit 1)
grep "OpenHit" test_logging_work/full_log.txt || (echo "OpenHit event not found"; exit 1)

echo "--- Verification Successful ---"
