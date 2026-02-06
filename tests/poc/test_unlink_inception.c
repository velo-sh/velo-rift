#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
  if (argc < 2) {
    fprintf(stderr, "Usage: %s <path>\n", argv[0]);
    return 1;
  }

  const char *path = argv[1];
  int res = unlink(path);
  if (res == 0) {
    printf("unlink SUCCESS (This is a bug if path is VFS)\n");
    return 0;
  } else {
    printf("unlink FAILED: %s (errno=%d)\n", strerror(errno), errno);
    return 0;
  }
}
