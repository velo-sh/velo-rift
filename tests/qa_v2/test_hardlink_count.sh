#!/bin/bash
# ==============================================================================
# Test: Hardlink Count / EINVAL regression
# ==============================================================================
# Tests that hardlinks inside and across VFS boundaries behave correctly.
#
# Known issue: link/linkat returns EINVAL instead of the expected EXDEV
# for cross-boundary links, or may incorrectly fail for same-boundary links.
#
# Expected:
#   - link() within VFS boundary → EXDEV (VFS protects integrity)
#   - link() cross-boundary (VFS→non-VFS or vice versa) → EXDEV
#   - link() entirely outside VFS → passes through to real syscall
#   - Correct errno (EXDEV, not EINVAL)
# ==============================================================================

source "$(dirname "${BASH_SOURCE[0]}")/test_setup.sh"

check_prerequisites || exit 1

log_section "Hardlink Count / EINVAL Regression"

# NOTE: No daemon needed — mutation blocking uses shim's VFS prefix check.

# Create test files
VFS_FILE="$TEST_WORKSPACE/src/hardlink_source.txt"
echo "hardlink content" > "$VFS_FILE"

OUTSIDE_DIR="/tmp/vrift_hardlink_test_$$"
mkdir -p "$OUTSIDE_DIR"
OUTSIDE_FILE="$OUTSIDE_DIR/outside_source.txt"
echo "outside content" > "$OUTSIDE_FILE"

# Compile hardlink probe
PROBE_SRC="$TEST_WORKSPACE/hardlink_probe.c"
PROBE_BIN="$TEST_WORKSPACE/hardlink_probe"

cat > "$PROBE_SRC" << 'PROBE_EOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>

// Check nlink count for a path
void check_nlink(const char *label, const char *path) {
    struct stat st;
    if (stat(path, &st) == 0) {
        printf("%s: nlink=%d\n", label, (int)st.st_nlink);
    } else {
        printf("%s: stat_errno=%d (%s)\n", label, errno, strerror(errno));
    }
}

int main(int argc, char *argv[]) {
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <vfs_file> <outside_file>\n", argv[0]);
        return 99;
    }
    const char *vfs_path = argv[1];
    const char *outside_path = argv[2];
    int ret;

    // Pre-test: check nlink before any link attempts
    check_nlink("pre_nlink_vfs", vfs_path);
    check_nlink("pre_nlink_outside", outside_path);

    // Test 1: link() within VFS (src=VFS, dst=VFS)
    char vfs_dst[2048];
    snprintf(vfs_dst, sizeof(vfs_dst), "%s.hardlink", vfs_path);
    errno = 0;
    ret = link(vfs_path, vfs_dst);
    printf("link_vfs_to_vfs: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 2: link() cross-boundary (src=VFS, dst=outside)
    char cross_dst[2048];
    snprintf(cross_dst, sizeof(cross_dst), "%s.from_vfs", outside_path);
    errno = 0;
    ret = link(vfs_path, cross_dst);
    printf("link_vfs_to_outside: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 3: link() cross-boundary (src=outside, dst=VFS)
    char cross_dst2[2048];
    snprintf(cross_dst2, sizeof(cross_dst2), "%s.from_outside", vfs_path);
    errno = 0;
    ret = link(outside_path, cross_dst2);
    printf("link_outside_to_vfs: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 4: linkat() within VFS
    char linkat_dst[2048];
    snprintf(linkat_dst, sizeof(linkat_dst), "%s.linkat", vfs_path);
    errno = 0;
    ret = linkat(AT_FDCWD, vfs_path, AT_FDCWD, linkat_dst, 0);
    printf("linkat_vfs_to_vfs: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 5: linkat() cross-boundary (src=VFS, dst=outside)
    char linkat_cross[2048];
    snprintf(linkat_cross, sizeof(linkat_cross), "%s.linkat_from_vfs", outside_path);
    errno = 0;
    ret = linkat(AT_FDCWD, vfs_path, AT_FDCWD, linkat_cross, 0);
    printf("linkat_vfs_to_outside: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Test 6: link() entirely outside VFS
    char outside_dst[2048];
    snprintf(outside_dst, sizeof(outside_dst), "%s.linked", outside_path);
    errno = 0;
    ret = link(outside_path, outside_dst);
    printf("link_outside_only: ret=%d errno=%d (%s)\n", ret, errno, strerror(errno));

    // Post-test: check nlink after
    check_nlink("post_nlink_vfs", vfs_path);
    check_nlink("post_nlink_outside", outside_path);

    // Cleanup outside links
    unlink(outside_dst);
    unlink(cross_dst);
    unlink(linkat_cross);

    return 0;
}
PROBE_EOF

cc -o "$PROBE_BIN" "$PROBE_SRC" -Wall 2>/dev/null || {
    log_fail "Failed to compile hardlink probe"
    exit_with_summary
}

# Run probe under shim
OUTPUT=$(run_with_shim "$PROBE_BIN" "$VFS_FILE" "$OUTSIDE_FILE" 2>&1)
echo "$OUTPUT"
echo ""

# ============================================================================
# Evaluate results
# ============================================================================

check_link_result() {
    local name="$1"
    local testid="$2"
    local desc="$3"
    local expected_errno="$4"  # Expected errno (18=EXDEV, 1=EPERM, or "18,1" for multiple)

    log_test "$testid" "$desc"
    local line
    line=$(echo "$OUTPUT" | grep "^${name}:")

    if [ -z "$line" ]; then
        log_fail "$name — not in output"
        return
    fi

    local actual_errno
    actual_errno=$(echo "$line" | sed 's/.*errno=\([0-9]*\).*/\1/')
    local actual_ret
    actual_ret=$(echo "$line" | sed 's/.*ret=\(-*[0-9]*\).*/\1/')

    # Support comma-separated expected errnos
    local errno_match=0
    for exp in $(echo "$expected_errno" | tr ',' ' '); do
        if [ "$actual_errno" = "$exp" ]; then
            errno_match=1
            break
        fi
    done

    if [ "$actual_ret" = "-1" ] && [ "$errno_match" = "1" ]; then
        log_pass "$name → errno=$actual_errno (expected)"
    elif [ "$actual_ret" = "-1" ] && [ "$actual_errno" = "22" ]; then
        log_fail "$name → EINVAL (errno=22) — REGRESSION: should be EXDEV(18) or EPERM(1), not EINVAL"
    elif [ "$actual_ret" = "-1" ]; then
        log_fail "$name → errno=$actual_errno (expected $expected_errno)"
    elif [ "$actual_ret" = "0" ] && [ "$expected_errno" != "0" ]; then
        log_fail "$name → SUCCEEDED but should have been blocked"
    else
        log_pass "$name → success (passthrough expected)"
    fi
}

# VFS→VFS link should be blocked (EXDEV or EPERM)
check_link_result "link_vfs_to_vfs"       "HLINK.1" "link(VFS→VFS) returns EXDEV/EPERM" "18,1"
# Cross-boundary should be blocked (EXDEV)
check_link_result "link_vfs_to_outside"   "HLINK.2" "link(VFS→outside) returns EXDEV"  "18"
check_link_result "link_outside_to_vfs"   "HLINK.3" "link(outside→VFS) returns EXDEV"  "18"
# linkat within VFS should be blocked (EXDEV or EPERM)
check_link_result "linkat_vfs_to_vfs"     "HLINK.4" "linkat(VFS→VFS) returns EXDEV/EPERM" "18,1"
check_link_result "linkat_vfs_to_outside" "HLINK.5" "linkat(VFS→outside) returns EXDEV" "18"
# Outside-only link should pass through
check_link_result "link_outside_only"     "HLINK.6" "link(outside→outside) passes through" "0"

# Check that VFS nlink did NOT increase
log_test "HLINK.7" "VFS file nlink unchanged after blocked links"
PRE_NLINK=$(echo "$OUTPUT" | grep "^pre_nlink_vfs:" | sed 's/.*nlink=\([0-9]*\)/\1/')
POST_NLINK=$(echo "$OUTPUT" | grep "^post_nlink_vfs:" | sed 's/.*nlink=\([0-9]*\)/\1/')
if [ "$PRE_NLINK" = "$POST_NLINK" ]; then
    log_pass "VFS nlink unchanged ($PRE_NLINK → $POST_NLINK)"
else
    log_fail "VFS nlink changed ($PRE_NLINK → $POST_NLINK) — integrity violation"
fi

# Cleanup
rm -rf "$OUTSIDE_DIR"

exit_with_summary
