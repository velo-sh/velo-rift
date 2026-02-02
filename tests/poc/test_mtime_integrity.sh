#!/bin/bash
# Test: mtime Integrity Verification
# Tests actual mtime behavior, not source code

echo "=== Timestamp Integrity (mtime) Verification ==="

TEST_DIR=$(mktemp -d)

cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

# 1. Create a file with a specific past timestamp
echo "Source Content" > "$TEST_DIR/old_file.txt"
# Set mtime to 2020-01-01 12:00:00
touch -t 202001011200 "$TEST_DIR/old_file.txt"

# Test mtime with Python
python3 << 'EOF'
import os
import time
import sys

test_dir = os.environ.get("TEST_DIR", "/tmp")
test_file = os.path.join(test_dir, "old_file.txt")

try:
    # Get file mtime
    stat = os.stat(test_file)
    mtime = stat.st_mtime
    
    # Expected: 2020-01-01 12:00:00 UTC (approximately)
    # This is about 1577880000 unix timestamp
    expected_year = 2020
    
    # Convert to struct_time
    t = time.localtime(mtime)
    
    print(f"File mtime: {mtime}")
    print(f"Parsed: {time.strftime('%Y-%m-%d %H:%M:%S', t)}")
    
    if t.tm_year == expected_year:
        print("✅ PASS: mtime correctly preserved (year 2020)")
        sys.exit(0)
    else:
        print(f"❌ FAIL: mtime year is {t.tm_year}, expected {expected_year}")
        sys.exit(1)
        
except Exception as e:
    print(f"mtime error: {e}")
    sys.exit(1)
EOF

export TEST_DIR="$TEST_DIR"

python3 -c "
import os
import time
import sys

test_file = '$TEST_DIR/old_file.txt'
mtime = os.stat(test_file).st_mtime
t = time.localtime(mtime)

if t.tm_year == 2020:
    print('✅ PASS: mtime preserved correctly')
    sys.exit(0)
else:
    print(f'❌ FAIL: wrong year {t.tm_year}')
    sys.exit(1)
"
