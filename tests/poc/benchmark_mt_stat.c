#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <time.h>
#include <pthread.h>

#define ITERATIONS 500000
#define NUM_THREADS 8

static inline long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

static int global_fd;

void *bench_thread(void *arg) {
    struct stat sb;
    for (int i = 0; i < ITERATIONS; i++) {
        fstat(global_fd, &sb);
    }
    return NULL;
}

int main(int argc, char **argv) {
    global_fd = open("/dev/null", O_RDONLY);
    if (global_fd < 0) { perror("open"); return 1; }

    pthread_t threads[NUM_THREADS];
    long long start = now_ns();

    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_create(&threads[i], NULL, bench_thread, NULL);
    }

    for (int i = 0; i < NUM_THREADS; i++) {
        pthread_join(threads[i], NULL);
    }

    long long end = now_ns();
    double total_time_s = (double)(end - start) / 1e9;
    double calls_per_sec = (double)(ITERATIONS * NUM_THREADS) / total_time_s;
    double ns_per_call = (double)(end - start) / (ITERATIONS * NUM_THREADS);

    printf("Throughtput: %.2f M calls/sec\n", calls_per_sec / 1e6);
    printf("Avg Latency (MT): %.2f ns/call\n", ns_per_call);

    close(global_fd);
    return 0;
}
