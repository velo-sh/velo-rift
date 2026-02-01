#!/bin/bash
# Test: Node.js/Bun Runtime Filesystem Analysis
# Goal: Analyze Node.js module resolution filesystem operations

set -e
echo "=== Node.js/Bun Runtime Filesystem Analysis ==="
echo ""

# Check runtimes
echo "[1] Runtime detection:"
if command -v node &> /dev/null; then
    echo "    Node.js: $(node --version)"
else
    echo "    Node.js: Not installed"
fi

if command -v bun &> /dev/null; then
    echo "    Bun: $(bun --version)"
else
    echo "    Bun: Not installed"
fi

echo ""
echo "[2] Package manager detection:"
npm --version 2>/dev/null && echo "    npm: $(npm --version)" || echo "    npm: Not installed"
pnpm --version 2>/dev/null && echo "    pnpm: $(pnpm --version)" || echo "    pnpm: Not installed"

echo ""
echo "[3] Node.js require() resolution algorithm:"
echo "    1. Check core modules (fs, path) - no FS"
echo "    2. Check require.cache - no FS"
echo "    3. Traverse node_modules directories:"
echo "       - stat(cwd/node_modules/pkg)"
echo "       - stat(../node_modules/pkg)"
echo "       - ... until root"
echo "    4. For each candidate:"
echo "       - stat(pkg/package.json)"
echo "       - open/read package.json"
echo "       - stat(pkg/main.js)"
echo "    5. Execute module"
echo ""
echo "    ⚠️ Heavy stat() usage per require()!"

echo ""
echo "[4] pnpm node_modules structure (optimal for VFS):"
echo ""
echo "    node_modules/"
echo "    ├── .pnpm/                          # Content-addressable"
echo "    │   └── lodash@4.17.21/"
echo "    │       └── node_modules/"
echo "    │           └── lodash/             # ← Hardlinked from store!"
echo "    └── lodash → .pnpm/.../lodash       # ← Symlink"
echo ""
echo "    🌟 Perfect match with Velo Rift CAS architecture!"

echo ""
echo "[5] VFS Compatibility Matrix:"
echo ""
echo "    ┌────────────────┬──────────┬────────────────┐"
echo "    │ Operation      │ VFS      │ Status         │"
echo "    ├────────────────┼──────────┼────────────────┤"
echo "    │ stat           │ ❌       │ Recursion bug  │"
echo "    │ readdir        │ ✅       │ Implemented    │"
echo "    │ readlink       │ ✅       │ Implemented    │"
echo "    │ open/read      │ ✅       │ Works          │"
echo "    │ hardlink       │ ✅       │ CAS strategy   │"
echo "    │ dlopen (.node) │ ❌       │ Not intercepted│"
echo "    └────────────────┴──────────┴────────────────┘"

echo ""
echo "[6] Strategic Opportunities:"
echo "    • VFS as pnpm store backend"
echo "    • VFS as Bun global cache"
echo "    • Pre-populated node_modules projection"

echo ""
echo "[7] Key Insight: pnpm/Bun use SAME pattern as Velo Rift!"
echo "    - Content-addressable global store"
echo "    - Hardlink to project"
echo "    - Symlink for structure"
