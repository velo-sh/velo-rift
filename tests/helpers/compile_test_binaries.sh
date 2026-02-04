#!/bin/bash
# compile_test_binaries.sh
# Compiles arm64-compatible replacement binaries for common shell commands
#
# PROBLEM: macOS system binaries (/bin/*) are arm64e with Pointer Authentication
#          which prevents DYLD_INSERT_LIBRARIES from working.
#
# SOLUTION: Compile simple C wrappers for each command - they will be arm64 and
#           DYLD injection works properly on arm64 binaries.
#
# Usage: source compile_test_binaries.sh $TEST_DIR/bin
#        This creates: $TEST_DIR/bin/echo, $TEST_DIR/bin/chmod, etc.

set -e

if [ -z "$1" ]; then
    echo "Usage: $0 <output_dir>"
    exit 1
fi

BIN_DIR="$1"
mkdir -p "$BIN_DIR"

# Temporary directory for C source files
SRC_DIR=$(mktemp -d)
trap "rm -rf $SRC_DIR" EXIT

echo "Compiling arm64-compatible test binaries to $BIN_DIR..."

# echo - simple echo replacement
cat > "$SRC_DIR/echo.c" << 'EOF'
#include <stdio.h>
int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        printf("%s%s", argv[i], i < argc - 1 ? " " : "");
    }
    printf("\n");
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/echo" "$SRC_DIR/echo.c"

# cat - simple cat replacement
cat > "$SRC_DIR/cat.c" << 'EOF'
#include <stdio.h>
int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        FILE *f = fopen(argv[i], "r");
        if (!f) { perror(argv[i]); return 1; }
        int c;
        while ((c = fgetc(f)) != EOF) putchar(c);
        fclose(f);
    }
    if (argc == 1) {
        int c;
        while ((c = getchar()) != EOF) putchar(c);
    }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/cat" "$SRC_DIR/cat.c"

# chmod - permission change
cat > "$SRC_DIR/chmod.c" << 'EOF'
#include <sys/stat.h>
#include <stdlib.h>
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: chmod mode file\n"); return 1; }
    mode_t mode = strtol(argv[1], NULL, 8);
    for (int i = 2; i < argc; i++) {
        if (chmod(argv[i], mode) < 0) { perror(argv[i]); return 1; }
    }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/chmod" "$SRC_DIR/chmod.c"

# rm - remove files
cat > "$SRC_DIR/rm.c" << 'EOF'
#include <unistd.h>
#include <stdio.h>
int main(int argc, char **argv) {
    int force = 0, i = 1;
    if (argc > 1 && argv[1][0] == '-' && argv[1][1] == 'f') { force = 1; i = 2; }
    for (; i < argc; i++) {
        if (unlink(argv[i]) < 0 && !force) { perror(argv[i]); return 1; }
    }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/rm" "$SRC_DIR/rm.c"

# mv - move/rename files
cat > "$SRC_DIR/mv.c" << 'EOF'
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc != 3) { fprintf(stderr, "usage: mv src dst\n"); return 1; }
    if (rename(argv[1], argv[2]) < 0) { perror("rename"); return 1; }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/mv" "$SRC_DIR/mv.c"

# cp - copy files (simple version)
cat > "$SRC_DIR/cp.c" << 'EOF'
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
int main(int argc, char **argv) {
    if (argc != 3) { fprintf(stderr, "usage: cp src dst\n"); return 1; }
    int src = open(argv[1], O_RDONLY);
    if (src < 0) { perror(argv[1]); return 1; }
    int dst = open(argv[2], O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (dst < 0) { perror(argv[2]); close(src); return 1; }
    char buf[8192];
    ssize_t n;
    while ((n = read(src, buf, sizeof(buf))) > 0) write(dst, buf, n);
    close(src); close(dst);
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/cp" "$SRC_DIR/cp.c"

# ln - create links
cat > "$SRC_DIR/ln.c" << 'EOF'
#include <unistd.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
    int symbolic = 0, i = 1;
    if (argc > 1 && strcmp(argv[1], "-s") == 0) { symbolic = 1; i = 2; }
    if (argc - i != 2) { fprintf(stderr, "usage: ln [-s] src dst\n"); return 1; }
    int ret = symbolic ? symlink(argv[i], argv[i+1]) : link(argv[i], argv[i+1]);
    if (ret < 0) { perror("ln"); return 1; }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/ln" "$SRC_DIR/ln.c"

# mkdir - create directory
cat > "$SRC_DIR/mkdir.c" << 'EOF'
#include <sys/stat.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
    int i = 1;
    if (argc > 1 && strcmp(argv[1], "-p") == 0) i = 2;
    for (; i < argc; i++) {
        if (mkdir(argv[i], 0755) < 0) { perror(argv[i]); return 1; }
    }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/mkdir" "$SRC_DIR/mkdir.c"

# touch - update mtime  
cat > "$SRC_DIR/touch.c" << 'EOF'
#include <sys/stat.h>
#include <sys/time.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        // Create file if doesn't exist
        int fd = open(argv[i], O_WRONLY | O_CREAT, 0644);
        if (fd >= 0) close(fd);
        // Update mtime
        if (utimes(argv[i], NULL) < 0) { perror(argv[i]); return 1; }
    }
    return 0;
}
EOF
cc -O2 -o "$BIN_DIR/touch" "$SRC_DIR/touch.c"

# Sign all binaries for DYLD_INSERT_LIBRARIES (macOS only)
if [ "$(uname -s)" == "Darwin" ]; then
    for bin in "$BIN_DIR"/*; do
        codesign -s - -f "$bin" 2>/dev/null || true
    done
fi

echo "Done. Created: echo, cat, chmod, rm, mv, cp, ln, mkdir, touch"
