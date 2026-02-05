#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

#ifdef __APPLE__
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/uio.h>
#endif

void test_futimes(int fd) {
  printf("Testing futimes on FD %d...\n", fd);
  struct timeval times[2];
  gettimeofday(&times[0], NULL);
  gettimeofday(&times[1], NULL);
  if (futimes(fd, times) == -1) {
    printf("futimes failed as expected: %s\n", strerror(errno));
  } else {
    printf("futimes SUCCEEDED (SHOULD HAVE FAILED for VFS!)\n");
    exit(1);
  }
}

#ifdef __APPLE__
void test_fchflags(int fd) {
  printf("Testing fchflags on FD %d...\n", fd);
  if (fchflags(fd, UF_NODUMP) == -1) {
    printf("fchflags failed as expected: %s\n", strerror(errno));
  } else {
    printf("fchflags SUCCEEDED (SHOULD HAVE FAILED for VFS!)\n");
    exit(1);
  }
}

void test_sendfile(int out_fd) {
  printf("Testing sendfile on FD %d (drain)...\n", out_fd);
  int in_fd = open("/etc/passwd", O_RDONLY);
  if (in_fd < 0) {
    perror("open /etc/passwd");
    return;
  }
  off_t len = 10;
  if (sendfile(in_fd, out_fd, 0, &len, NULL, 0) == -1) {
    printf("sendfile failed as expected: %s\n", strerror(errno));
  } else {
    printf("sendfile SUCCEEDED (SHOULD HAVE FAILED for VFS!)\n");
    exit(1);
  }
  close(in_fd);
}
#endif

int main(int argc, char *argv[]) {
  if (argc < 3) {
    fprintf(stderr, "Usage: %s <test_type> <path>\n", argv[0]);
    return 1;
  }

  const char *test_type = argv[1];
  const char *path = argv[2];

  int fd = open(path, O_RDWR);
  if (fd < 0) {
    // Try O_RDONLY if O_RDWR fails (for futimes/fchflags blocks)
    fd = open(path, O_RDONLY);
  }

  if (fd < 0) {
    perror("open test file");
    return 1;
  }

  if (strcmp(test_type, "futimes") == 0) {
    test_futimes(fd);
  }
#ifdef __APPLE__
  else if (strcmp(test_type, "fchflags") == 0) {
    test_fchflags(fd);
  } else if (strcmp(test_type, "sendfile") == 0) {
    test_sendfile(fd);
  }
#endif
  else {
    fprintf(stderr, "Unknown test type: %s\n", test_type);
    close(fd);
    return 1;
  }

  close(fd);
  return 0;
}
