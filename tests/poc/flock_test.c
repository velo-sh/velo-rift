#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/file.h>
#include <sys/time.h>
#include <errno.h>

long current_ms() {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec * 1000 + tv.tv_usec / 1000;
}

int main(int argc, char *argv[]) {
    if (argc < 4) {
        fprintf(stderr, "Usage: %s <file> <op> <sleep_ms>\n", argv[0]);
        return 1;
    }

    const char *path = argv[1];
    int op = atoi(argv[2]); // 2=EX, 1=SH, 8=UN
    int sleep_ms = atoi(argv[3]);

    int fd = open(path, O_RDWR | O_CREAT, 0666);
    if (fd < 0) {
        perror("open");
        return 1;
    }

    // printf("PID %d: Acquiring lock...\n", getpid());
    long t0 = current_ms();
    if (flock(fd, op) != 0) {
        perror("flock");
        return 1;
    }
    long t1 = current_ms();
    printf("PID %d: Acquired lock in %ld ms\n", getpid(), t1 - t0);

    if (sleep_ms > 0) {
        usleep(sleep_ms * 1000);
    }

    flock(fd, LOCK_UN);
    close(fd);
    return 0;
}
