#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <file>\n", argv[0]);
        return 1;
    }

    const char *path = argv[1];
    
    // 1. Open file (should trigger VFS CoW)
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        perror("open");
        return 1;
    }

    // 2. mmap (MAP_SHARED)
    // Map 4KB or file size
    size_t len = 4096;
    void *addr = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (addr == MAP_FAILED) {
        perror("mmap");
        return 1;
    }

    // 3. Write updates
    const char *msg = "UPDATED_BY_MMAP";
    memcpy(addr, msg, strlen(msg));

    // 4. Unmap (Should trigger reingest)
    if (munmap(addr, len) != 0) {
        perror("munmap");
        return 1;
    }

    // 5. Close
    close(fd);
    return 0;
}
