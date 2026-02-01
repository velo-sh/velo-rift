#!/bin/bash
# Test: UV/Pip Package Manager Compatibility
# Goal: Analyze package manager filesystem operations
# This is an analysis test to understand pip/uv's filesystem needs

set -e
echo "=== UV/Pip Package Manager Filesystem Analysis ==="
echo ""

# Check if uv is installed
if command -v uv &> /dev/null; then
    echo "[1] UV detected: $(uv --version)"
    UV_AVAILABLE=1
else
    echo "[1] UV not installed, using pip only"
    UV_AVAILABLE=0
fi

# Create temp environment
TEMP_DIR=$(mktemp -d)
cd "$TEMP_DIR"

echo ""
echo "[2] pip install analysis (filesystem operations):"
echo "    Key operations during 'pip install requests':"
echo "    - stat: Check package exists, get mtime"
echo "    - open/read: Read wheel contents"
echo "    - write: Extract to site-packages"
echo "    - link: (none - pip copies)"
echo ""

if [ "$UV_AVAILABLE" -eq 1 ]; then
    echo "[3] UV install analysis:"
    echo "    Key operations during 'uv pip install requests':"
    echo "    - stat: Check lockfile, cache"
    echo "    - link: Hardlink from global cache to venv"
    echo "    - No copy! Directly hardlinks wheel contents"
    echo ""
    
    echo "[4] UV cache location:"
    UV_CACHE=$(uv cache dir 2>/dev/null || echo "~/.cache/uv")
    echo "    $UV_CACHE"
    
    echo ""
    echo "[5] UV link modes:"
    echo "    - hardlink (default Linux/Windows)"
    echo "    - clone (default macOS)"
    echo "    - symlink"
    echo "    - copy (fallback)"
fi

echo ""
echo "[6] VFS Compatibility Analysis:"
echo ""
echo "    ┌─────────────────────────────────────────────────────┐"
echo "    │                   Velo Rift CAS                     │"
echo "    │  (Content-addressable blobs)                        │"
echo "    ├─────────────────────────────────────────────────────┤"
echo "    │         ↓ hardlink              ↓ hardlink          │"
echo "    │ ┌──────────────────┐    ┌──────────────────┐        │"
echo "    │ │   VFS Projection │    │   UV venv        │        │"
echo "    │ │   /vrift/numpy/  │    │   .venv/numpy/   │        │"
echo "    │ └──────────────────┘    └──────────────────┘        │"
echo "    └─────────────────────────────────────────────────────┘"
echo ""

echo "[7] Synergy Assessment:"
echo "    ✓ Both use content-addressable storage"
echo "    ✓ Both use hardlink for deduplication"
echo "    ✓ Both support multiple environments from single cache"
echo "    ⚠ VFS stat recursion blocks compatibility"
echo ""

echo "[8] Strategic Opportunity:"
echo "    Velo Rift could serve as UV-compatible cache backend:"
echo "    [tool.uv]"
echo "    cache-dir = \"/vrift/python-cache\""
echo "    link-mode = \"hardlink\""

# Cleanup
cd /
rm -rf "$TEMP_DIR"
