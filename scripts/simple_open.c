#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
  if (argc < 2) {
    fprintf(stderr, "Usage: %s <path|iterations> [iterations]\n", argv[0]);
    return 1;
  }

  // Case 1: simple_open <path>
  // Case 2: simple_open <path> <iterations>
  // Case 3: simple_open <iterations> (for logging test)

  char *path = "/tmp/vrift_simple_open.txt";
  int iterations = 1;

  if (argc == 2) {
    if (atoi(argv[1]) > 0) {
      iterations = atoi(argv[1]);
    } else {
      path = argv[1];
    }
  } else if (argc == 3) {
    path = argv[1];
    iterations = atoi(argv[2]);
  }

  printf("Starting simple_open: path=%s iterations=%d\n", path, iterations);

  for (int i = 0; i < iterations; i++) {
    int fd = open(path, O_RDONLY | O_CREAT, 0644);
    if (fd < 0) {
      perror("open");
    } else {
      printf("Open iteration %d successful: fd=%d\n", i, fd);
      close(fd);
    }
    if (iterations > 1) {
      sleep(1);
    }
  }

  return 0;
}
