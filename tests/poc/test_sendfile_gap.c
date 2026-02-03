#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>

#ifdef __APPLE__
#include <sys/socket.h>
#include <sys/uio.h>
#else
#include <sys/sendfile.h>
#endif

int main(int argc, char *argv[]) {
  if (argc < 3) {
    fprintf(stderr, "Usage: %s <src> <dest>\n", argv[0]);
    return 1;
  }

  const char *src_path = argv[1];
  const char *dest_path = argv[2];

  int src_fd = open(src_path, O_RDONLY);
  if (src_fd < 0) {
    perror("open src");
    return 1;
  }

  int dest_fd = open(dest_path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
  if (dest_fd < 0) {
    perror("open dest");
    close(src_fd);
    return 1;
  }

  off_t offset = 0;
  int res;

#ifdef __APPLE__
  off_t len = 0; // send all
  res = sendfile(src_fd, dest_fd, 0, &len, NULL, 0);
#else
  res = sendfile(dest_fd, src_fd, &offset, 4096);
#endif

  if (res == 0) {
    printf("sendfile SUCCESS (This is a gap if dest is VFS)\n");
    close(src_fd);
    close(dest_fd);
    return 0;
  } else {
    printf("sendfile FAILED: %s (errno=%d)\n", strerror(errno), errno);
    close(src_fd);
    close(dest_fd);
    return 0;
  }
}
