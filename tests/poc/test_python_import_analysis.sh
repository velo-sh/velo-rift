#!/bin/bash
# Test: Python Import Syscall Analysis
# Goal: Trace what syscalls Python actually uses during import
# This is an analysis test to understand Python's filesystem needs

set -e
echo "=== Python Import Syscall Analysis ==="
echo ""

# Create temp module
TEMP_DIR=$(mktemp -d)
mkdir -p "$TEMP_DIR/mypackage"

cat > "$TEMP_DIR/mypackage/__init__.py" << 'EOF'
print("mypackage __init__ loaded")
EOF

cat > "$TEMP_DIR/mypackage/utils.py" << 'EOF'
def hello():
    return "Hello from utils"
EOF

echo "[1] Import trace (using built-in tracing)"
PYTHONPATH="$TEMP_DIR" python3 -c "
import sys
import os

# Show import path
print('sys.path:', sys.path[:3], '...')

# Import and trace
print('\\n--- Importing mypackage ---')
import mypackage
print('mypackage loaded from:', mypackage.__file__)

print('\\n--- Importing mypackage.utils ---')
from mypackage import utils
print('utils loaded from:', utils.__file__)

# Show __pycache__ creation
pycache = os.path.join('$TEMP_DIR', 'mypackage', '__pycache__')
if os.path.exists(pycache):
    print('\\n__pycache__ contents:')
    for f in os.listdir(pycache):
        print(f'  {f}')
"

echo ""
echo "[2] Key observations for VFS:"
echo "  - Python uses stat() to check if .py exists and get mtime"
echo "  - Python uses stat() to check __pycache__/*.pyc mtime"
echo "  - Python creates __pycache__/ on first import (write operation)"
echo "  - Import order: stat → open → read → (compile) → execute"
echo ""
echo "[3] macOS dtruss example (requires sudo):"
echo "  sudo dtruss -f python3 -c 'import json' 2>&1 | grep -E 'stat|open|mmap'"

# Cleanup
rm -rf "$TEMP_DIR"
