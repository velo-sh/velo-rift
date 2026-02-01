#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/**
 * verify_abi_hazard.c
 *
 * This test verifies that variadic syscalls like fcntl and open are called
 * correctly according to Apple's ARM64 ABI (arguments on stack).
 *
 * If the shim fails to pass arguments on the stack, fcntl(F_DUPFD_CLOEXEC)
 * will return EINVAL (Invalid Argument) because it reads garbage from the
 * stack.
 */

int main() {
  printf("Starting ABI Hazard Verification...\n");

  // --- TEST 1: fcntl F_DUPFD_CLOEXEC (67) (Variadic, 3rd arg is usize) ---
  int fd = open("/dev/null", O_RDONLY);
  if (fd < 0) {
    perror("FAILED: open /dev/null");
    return 1;
  }

  printf("[Test 1] Invoking fcntl(fd, 67, 100)...\n");
  int new_fd = fcntl(fd, 67, 100);

  if (new_fd < 0) {
    printf("FAILED: fcntl F_DUPFD_CLOEXEC returned errno %d (%s)\n", errno,
           strerror(errno));
    if (errno == EINVAL) {
      printf("CRITICAL: Detected EINVAL - This usually indicates an ABI "
             "mismatch (arg not on stack).\n");
    }
    close(fd);
    return 1;
  } else {
    printf("SUCCESS: fcntl F_DUPFD_CLOEXEC returned fd %d\n", new_fd);
    close(new_fd);
  }
  close(fd);

  // --- TEST 2: open O_CREAT (Variadic, 3rd arg is mode_t) ---
  const char *test_file = "/tmp/vrift_abi_test.txt";
  unlink(test_file);

  printf("[Test 2] Invoking open(\"%s\", O_CREAT | O_WRONLY, 0644)...\n",
         test_file);
  int fd2 = open(test_file, O_CREAT | O_WRONLY, 0644);

  if (fd2 < 0) {
    printf("FAILED: open O_CREAT failed with errno %d (%s)\n", errno,
           strerror(errno));
    return 1;
  } else {
    printf("SUCCESS: open O_CREAT succeeded\n");
    close(fd2);
    struct stat st;
    if (stat(test_file, &st) == 0) {
      printf("File mode: %o\n", st.st_mode & 0777);
      if ((st.st_mode & 0777) != 0644) {
        printf("WARNING: File mode mismatch! Expected 644, got %o\n",
               st.st_mode & 0777);
      }
    }
    unlink(test_file);
  }

  printf("\n>>> ALL ABI HAZARD TESTS PASSED <<<\n");
  return 0;
}
