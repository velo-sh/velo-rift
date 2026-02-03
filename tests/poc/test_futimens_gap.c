#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
  if (argc < 2) {
    fprintf(stderr, "Usage: %s <path>\n", argv[0]);
    return 1;
  }

  const char *path = argv[1];
  int fd = open(path, O_RDONLY);
  if (fd < 0) {
    perror("open");
    return 1;
  }

  struct timespec times[2];
  times[0].tv_sec = 0; // access time
  times[0].tv_nsec = 0;
  times[1].tv_sec = 0; // modification time
  times[1].tv_nsec = 0;

  int res = futimens(fd, times);
  if (res == 0) {
    printf("futimens SUCCESS (This is a gap if path is VFS)\n");
    close(fd);
    return 0;
  } else {
    printf("futimens FAILED: %s (errno=%d)\n", strerror(errno), errno);
    close(fd);
    return 0;
  }
}
