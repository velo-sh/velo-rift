#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char *argv[]) {
  if (argc < 3) {
    printf("Usage: %s <old> <new>\n", argv[0]);
    return 1;
  }

  if (rename(argv[1], argv[2]) == 0) {
    printf("✅ Success: rename(%s, %s) ok\n", argv[1], argv[2]);
    return 0;
  } else {
    printf("❌ Failure: rename failed (errno=%d: %s)\n", errno, strerror(errno));
    return 0; // Exit 0 so bash doesn't stop, we'll check output
  }
}
