#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>

int main(int argc, char *argv[]) {
  if (argc < 2) {
    fprintf(stderr, "Usage: %s <path>\n", argv[0]);
    return 1;
  }

  const char *path = argv[1];
  int res = chmod(path, 0777);
  if (res == 0) {
    printf("chmod SUCCESS (This is a bug if path is VFS)\n");
    return 0;
  } else {
    printf("chmod FAILED: %s (errno=%d)\n", strerror(errno), errno);
    return 0;
  }
}
