#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
  if (argc < 3) {
    printf("Usage: %s <dir_path> <relative_path>\n", argv[0]);
    return 1;
  }

  const char *dir_path = argv[1];
  const char *rel_path = argv[2];

  int dir_fd = open(dir_path, O_RDONLY);
  if (dir_fd < 0) {
    perror("open dir");
    return 1;
  }

  printf("🧪 Attempting openat(%d [%s], \"%s\", O_RDONLY)...\n", dir_fd,
         dir_path, rel_path);
  int file_fd = openat(dir_fd, rel_path, O_RDONLY);

  if (file_fd >= 0) {
    printf("✅ Success: openat returned FD %d\n", file_fd);
    char buf[256];
    ssize_t n = read(file_fd, buf, sizeof(buf) - 1);
    if (n >= 0) {
      buf[n] = '\0';
      printf("   Content: %s\n", buf);
    } else {
      perror("   read failed");
    }
    close(file_fd);
  } else {
    printf("❌ Failure: openat failed (errno=%d: %s)\n", errno, strerror(errno));
  }

  close(dir_fd);
  return 0;
}
