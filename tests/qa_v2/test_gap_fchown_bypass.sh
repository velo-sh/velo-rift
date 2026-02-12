#!/bin/bash
# ==============================================================================
# Gap Test: fchown/fchownat bypass detection
# ==============================================================================
# Verifies that the shim blocks ownership changes (fchown, fchownat, lchown,
# chown) on VFS-managed files. The shim currently interposes fchown/fchownat
# via __interpose, but chown/lchown may be missing.
#
# Expected: All ownership mutations return EPERM for VFS files.
# ==============================================================================

source "$(dirname "${BASH_SOURCE[0]}")/test_setup.sh"

check_prerequisites || exit 1

log_section "Gap: fchown/fchownat Bypass Detection"

# NOTE: No daemon needed here — mutation blocking uses the shim's
# VFS prefix check (quick_block_vfs_mutation), not the daemon's VDir mmap.

# Create a test file inside VFS workspace
TEST_FILE="$TEST_WORKSPACE/src/owned_file.txt"
echo "owned content" > "$TEST_FILE"

# Compile the fchown probe
PROBE_SRC="$TEST_WORKSPACE/fchown_probe.c"
PROBE_BIN="/tmp/vrift_fchown_probe_$$"

cat > "$PROBE_SRC" << 'PROBE_EOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <path>\n", argv[0]);
        return 99;
    }
    const char *path = argv[1];
    int fd, ret;
    uid_t uid = getuid();
    gid_t gid = getgid();

    // Test 1: chown (path-based)
    errno = 0;
    ret = chown(path, uid, gid);
    printf("chown: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 2: lchown (path-based, no-follow)
    errno = 0;
    ret = lchown(path, uid, gid);
    printf("lchown: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 3: fchown (FD-based)
    fd = open(path, O_RDONLY);
    if (fd < 0) {
        printf("fchown: SKIP (cannot open fd)\n");
    } else {
        errno = 0;
        ret = fchown(fd, uid, gid);
        printf("fchown: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));
        close(fd);
    }

    // Test 4: fchownat (AT_FDCWD with absolute path)
    // NOTE: Using AT_FDCWD + absolute path so the shim can match the VFS prefix.
    // Using dirfd + relative basename would bypass prefix-based VFS detection.
    errno = 0;
    ret = fchownat(AT_FDCWD, path, uid, gid, 0);
    printf("fchownat: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    return 0;
}
PROBE_EOF

cc -o "$PROBE_BIN" "$PROBE_SRC" -Wall 2>/dev/null || {
    log_fail "Failed to compile fchown probe"
    exit_with_summary
}

# Run probe under shim
OUTPUT=$(run_with_shim "$PROBE_BIN" "$TEST_FILE" 2>&1)
echo "$OUTPUT"

# ============================================================================
# Evaluate results
# ============================================================================

# Test 1: chown on VFS file should return EPERM
log_test "G-FCHOWN.1" "chown() on VFS file returns EPERM"
if echo "$OUTPUT" | grep "^chown:" | grep -q "errno=1"; then
    log_pass "chown() blocked with EPERM"
else
    ACTUAL_ERRNO=$(echo "$OUTPUT" | grep "^chown:" | sed 's/.*errno=\([0-9]*\).*/\1/')
    log_fail "chown() NOT blocked (errno=$ACTUAL_ERRNO, expected 1/EPERM) — GAP: chown not interposed"
fi

# Test 2: lchown on VFS file should return EPERM
log_test "G-FCHOWN.2" "lchown() on VFS file returns EPERM"
if echo "$OUTPUT" | grep "^lchown:" | grep -q "errno=1"; then
    log_pass "lchown() blocked with EPERM"
else
    ACTUAL_ERRNO=$(echo "$OUTPUT" | grep "^lchown:" | sed 's/.*errno=\([0-9]*\).*/\1/')
    log_fail "lchown() NOT blocked (errno=$ACTUAL_ERRNO, expected 1/EPERM) — GAP: lchown not interposed"
fi

# Test 3: fchown on VFS file should return EPERM
log_test "G-FCHOWN.3" "fchown() on VFS file returns EPERM"
if echo "$OUTPUT" | grep "^fchown:" | grep -q "errno=1"; then
    log_pass "fchown() blocked with EPERM"
else
    ACTUAL_ERRNO=$(echo "$OUTPUT" | grep "^fchown:" | sed 's/.*errno=\([0-9]*\).*/\1/')
    log_fail "fchown() NOT blocked (errno=$ACTUAL_ERRNO, expected 1/EPERM) — IT_FCHOWN active but may be broken"
fi

# Test 4: fchownat on VFS file should return EPERM
log_test "G-FCHOWN.4" "fchownat() on VFS file returns EPERM"
if echo "$OUTPUT" | grep "^fchownat:" | grep -q "errno=1"; then
    log_pass "fchownat() blocked with EPERM"
else
    ACTUAL_ERRNO=$(echo "$OUTPUT" | grep "^fchownat:" | sed 's/.*errno=\([0-9]*\).*/\1/')
    log_fail "fchownat() NOT blocked (errno=$ACTUAL_ERRNO, expected 1/EPERM) — IT_FCHOWNAT may be broken"
fi

exit_with_summary
