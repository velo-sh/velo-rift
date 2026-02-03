#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
  if (argc < 3) {
    fprintf(stderr, "Usage: %s <target> <linkpath>\n", argv[0]);
    return 1;
  }

  const char *target = argv[1];
  const char *linkpath = argv[2];
  int res = symlinkat(target, AT_FDCWD, linkpath);
  if (res == 0) {
    printf("symlinkat SUCCESS (This is a bug if path is VFS)\n");
    return 0;
  } else {
    printf("symlinkat FAILED: %s (errno=%d)\n", strerror(errno), errno);
    return 0;
  }
}
