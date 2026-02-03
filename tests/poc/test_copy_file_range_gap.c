#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#ifndef __NR_copy_file_range
#define __NR_copy_file_range 285
#endif

int main(int argc, char *argv[]) {
#ifdef __APPLE__
  printf("copy_file_range N/A on macOS (Linux only)\n");
  return 0;
#else
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

  ssize_t res = copy_file_range(src_fd, NULL, dest_fd, NULL, 4096, 0);

  if (res >= 0) {
    printf("copy_file_range SUCCESS (This is a gap if dest is VFS)\n");
    close(src_fd);
    close(dest_fd);
    return 0;
  } else {
    printf("copy_file_range FAILED: %s (errno=%d)\n", strerror(errno), errno);
    close(src_fd);
    close(dest_fd);
    return 0;
  }
#endif
}
