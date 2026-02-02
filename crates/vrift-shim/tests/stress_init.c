#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

#define THREAD_COUNT 10
#define VFS_PATH "/vrift/stress_test_path"

void *thread_func(void *arg) {
  long id = (long)arg;
  struct stat st;

  // Attempt concurrent stat on first access
  if (stat(VFS_PATH, &st) == -1) {
    if (errno == ENOENT) {
      printf("[Thread %ld] BUG FOUND: Returned ENOENT (init race)\n", id);
      exit(1);
    }
  } else {
    printf("[Thread %ld] Success\n", id);
  }
  return NULL;
}

int main() {
  pthread_t threads[THREAD_COUNT];

  printf("Starting concurrent init stress test...\n");

  for (long i = 0; i < THREAD_COUNT; i++) {
    pthread_create(&threads[i], NULL, thread_func, (void *)i);
  }

  for (int i = 0; i < THREAD_COUNT; i++) {
    pthread_join(threads[i], NULL);
  }

  printf("Test completed successfully.\n");
  return 0;
}
