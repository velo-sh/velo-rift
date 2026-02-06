#!/bin/bash
# Shell-First Test Suite: All mutation operations MUST be intercepted
# This tests actual shell commands (cp, mv, rm, chmod, etc.) under the shim
# Uses local binary copies to bypass macOS SIP

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_DIR=$(mktemp -d)
VELO_PROJECT_ROOT="$TEST_DIR/workspace"

echo "=== Shell-First Test Suite: Mutation Operations ==="
echo "These tests verify that shell commands are properly intercepted"
echo "Build systems use: chmod, rm, mv, ln, touch, cp, etc."

# Prepare VFS workspace
mkdir -p "$VELO_PROJECT_ROOT/.vrift"
echo "PROTECTED" > "$VELO_PROJECT_ROOT/test.txt"
chmod 444 "$VELO_PROJECT_ROOT/test.txt"

# Copy and sign all utilities to bypass SIP/Security restrictions
# NOTE: macOS arm64e binaries (like /bin/*) don't work with DYLD_INSERT_LIBRARIES
# We compile simple arm64 replacements instead
mkdir -p "$TEST_DIR/bin"
HELPERS_DIR="$(cd "${SCRIPT_DIR}/../helpers" && pwd)"
if [ -f "$HELPERS_DIR/compile_test_binaries.sh" ]; then
    source "$HELPERS_DIR/compile_test_binaries.sh" "$TEST_DIR/bin"
else
    # Fallback: compile inline  
    for cmd in echo cat chmod rm mv cp mkdir touch; do
        case $cmd in
            echo)  echo '#include <stdio.h>
                   int main(int argc, char **argv) { for (int i=1;i<argc;i++) printf("%s%s",argv[i],i<argc-1?" ":""); printf("\n"); return 0; }' | cc -O2 -x c - -o "$TEST_DIR/bin/$cmd" ;;
            chmod) echo '#include <sys/stat.h>
                   #include <stdlib.h>
                   int main(int argc, char **argv) { if(argc<3)return 1; return chmod(argv[2],strtol(argv[1],0,8)); }' | cc -O2 -x c - -o "$TEST_DIR/bin/$cmd" ;;
            rm)    echo '#include <unistd.h>
                   int main(int argc, char **argv) { for(int i=1;i<argc;i++) unlink(argv[i]); return 0; }' | cc -O2 -x c - -o "$TEST_DIR/bin/$cmd" ;;
            mv)    echo '#include <stdio.h>
                   int main(int argc, char **argv) { return argc==3 ? rename(argv[1],argv[2]) : 1; }' | cc -O2 -x c - -o "$TEST_DIR/bin/$cmd" ;;
            *) continue ;;
        esac
        [ "$(uname -s)" == "Darwin" ] && codesign -s - -f "$TEST_DIR/bin/$cmd" 2>/dev/null || true
    done
fi

# Setup shim environment with local PATH
export PATH="$TEST_DIR/bin:$PATH"

if [[ "$(uname)" == "Darwin" ]]; then
    if [[ -f "${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib" ]]; then
        export SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_inception_layer.dylib"
    else
        export SHIM_LIB="${PROJECT_ROOT}/target/debug/libvrift_inception_layer.dylib"
    fi
    export SHIM_INJECT_VAR="DYLD_INSERT_LIBRARIES"
    export DYLD_FORCE_FLAT_NAMESPACE=1
else
    if [[ -f "${PROJECT_ROOT}/target/release/libvrift_shim.so" ]]; then
        export SHIM_LIB="${PROJECT_ROOT}/target/release/libvrift_shim.so"
    else
        export SHIM_LIB="${PROJECT_ROOT}/target/debug/libvrift_shim.so"
    fi
    export SHIM_INJECT_VAR="LD_PRELOAD"
fi

export $SHIM_INJECT_VAR="$SHIM_LIB"
export VRIFT_VFS_PREFIX="$VELO_PROJECT_ROOT"

FAILURES=0

# Test 1: chmod
echo -e "\n[Test 1] chmod 644"
if [[ "$(uname)" == "Darwin" ]]; then
    ORIGINAL=$(stat -f "%Lp" "$VELO_PROJECT_ROOT/test.txt")
else
    ORIGINAL=$(stat -c "%a" "$VELO_PROJECT_ROOT/test.txt")
fi

env "$SHIM_INJECT_VAR=$SHIM_LIB" "$TEST_DIR/bin/chmod" 644 "$VELO_PROJECT_ROOT/test.txt" 2>/dev/null

if [[ "$(uname)" == "Darwin" ]]; then
    NEW=$(stat -f "%Lp" "$VELO_PROJECT_ROOT/test.txt")
else
    NEW=$(stat -c "%a" "$VELO_PROJECT_ROOT/test.txt")
fi

if [[ "$ORIGINAL" != "$NEW" ]]; then
    echo "  ❌ FAIL: chmod bypassed (mode: $ORIGINAL -> $NEW)"
    ((FAILURES++))
else
    echo "  ✅ PASS: chmod blocked or virtualized"
fi

# Test 2: rm
echo -e "\n[Test 2] rm"
echo "DELETABLE" > "$VELO_PROJECT_ROOT/delete_me.txt"
if env "$SHIM_INJECT_VAR=$SHIM_LIB" "$TEST_DIR/bin/rm" "$VELO_PROJECT_ROOT/delete_me.txt" 2>/dev/null; then
    if [[ ! -f "$VELO_PROJECT_ROOT/delete_me.txt" ]]; then
        echo "  ❌ FAIL: rm bypassed (file deleted)"
        ((FAILURES++))
    else
        echo "  ✅ PASS: rm virtualized (file still exists on CAS)"
    fi
else
    echo "  ✅ PASS: rm blocked"
fi

# Test 3: mv (rename)
echo -e "\n[Test 3] mv (rename within VFS)"
echo "MOVABLE" > "$VELO_PROJECT_ROOT/move_me.txt"
if env "$SHIM_INJECT_VAR=$SHIM_LIB" "$TEST_DIR/bin/mv" "$VELO_PROJECT_ROOT/move_me.txt" "$VELO_PROJECT_ROOT/moved.txt" 2>/dev/null; then
    echo "  ⚠️  mv succeeded - check if virtualized or bypassed"
else
    echo "  ✅ PASS: mv blocked"
fi

# Test 4: cp (copy)
echo -e "\n[Test 4] cp (copy within VFS)"
echo "ORIGINAL" > "$VELO_PROJECT_ROOT/original.txt"
if env "$SHIM_INJECT_VAR=$SHIM_LIB" "$TEST_DIR/bin/cp" "$VELO_PROJECT_ROOT/original.txt" "$VELO_PROJECT_ROOT/copy.txt" 2>/dev/null; then
    if [[ -f "$VELO_PROJECT_ROOT/copy.txt" ]]; then
        echo "  ✅ PASS: cp succeeded (expected for read-only source)"
    fi
else
    echo "  ⚠️  cp blocked"
fi

# Test 5: touch (mtime modification)
echo -e "\n[Test 5] touch (mtime change)"
if [[ "$(uname)" == "Darwin" ]]; then
    ORIGINAL_MTIME=$(stat -f "%m" "$VELO_PROJECT_ROOT/test.txt")
else
    ORIGINAL_MTIME=$(stat -c "%Y" "$VELO_PROJECT_ROOT/test.txt")
fi

sleep 1
env "$SHIM_INJECT_VAR=$SHIM_LIB" "$TEST_DIR/bin/touch" "$VELO_PROJECT_ROOT/test.txt" 2>/dev/null

if [[ "$(uname)" == "Darwin" ]]; then
    NEW_MTIME=$(stat -f "%m" "$VELO_PROJECT_ROOT/test.txt")
else
    NEW_MTIME=$(stat -c "%Y" "$VELO_PROJECT_ROOT/test.txt")
fi

if [[ "$ORIGINAL_MTIME" != "$NEW_MTIME" ]]; then
    echo "  ❌ FAIL: touch bypassed (mtime changed)"
    ((FAILURES++))
else
    echo "  ✅ PASS: touch blocked or virtualized"
fi

# Cleanup
unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE
rm -rf "$TEST_DIR"

echo -e "\n=== Summary ==="
if [[ $FAILURES -gt 0 ]]; then
    echo "❌ $FAILURES test(s) FAILED - Shell commands bypass shim!"
    exit 1
else
    echo "✅ All shell commands properly intercepted or virtualized"
    exit 0
fi
