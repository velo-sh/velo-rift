#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main(int argc, char **argv) {
  signal(SIGPIPE, SIG_IGN);
  if (argc < 2) {
    fprintf(stderr, "Usage: %s <file>\n", argv[0]);
    return 1;
  }
  printf("Opening %s...\n", argv[1]);
  int fd = open(argv[1], O_RDONLY);
  if (fd < 0) {
    perror("open");
    return 1;
  }
  char buf[1024];
  ssize_t n = read(fd, buf, sizeof(buf) - 1);
  if (n < 0) {
    perror("read");
    close(fd);
    return 1;
  }
  buf[n] = '\0';
  printf("Content:\n%s\n", buf);
  close(fd);
  return 0;
}
