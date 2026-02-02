/**
 * Multi-Platform Variadic Shim
 * Supports: macOS (ARM64), Linux (x86_64, ARM64)
 *
 * This bridge solves the variadic ABI hazard and enables "Delayed VFS
 * Activation". It uses raw inline assembly for syscalls during the
 * initialization phase to prevent recursion and deadlocks.
 */

#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <unistd.h>

/* --- Platform Specific Syscall Numbers --- */

#if defined(__APPLE__) && defined(__aarch64__)
#define SYS_OPEN 5
#define SYS_OPENAT 463
#elif defined(__linux__) && defined(__x86_64__)
#define SYS_OPEN 2
#define SYS_OPENAT 257
#elif defined(__linux__) && defined(__aarch64__)
// Linux AArch64 often only has openat
#define SYS_OPENAT 56
#define AT_FDCWD -100
#endif

/* --- External Rust Implementation & Flags --- */

extern int velo_open_impl(const char *path, int flags, mode_t mode);
extern int velo_openat_impl(int dirfd, const char *path, int flags,
                            mode_t mode);

extern _Atomic char INITIALIZING; /* 1 = Busy/Initializing, 0 = Ready */

/* --- Raw Syscall Implementation --- */

#if defined(__aarch64__)
/**
 * ARM64 (AArch64) Raw Syscall
 */
static inline long raw_syscall(long number, long arg1, long arg2, long arg3,
                               long arg4) {
  long ret;
#if defined(__APPLE__)
  long err_flag;
  register long x16 __asm__("x16") = number;
  register long x0 __asm__("x0") = arg1;
  register long x1 __asm__("x1") = arg2;
  register long x2 __asm__("x2") = arg3;
  register long x3 __asm__("x3") = arg4;

  __asm__ volatile("svc #0x80\n"
                   "cset %1, cs\n"
                   : "+r"(x0), "=r"(err_flag)
                   : "r"(x16), "r"(x1), "r"(x2), "r"(x3)
                   : "memory");
  if (err_flag) {
    errno = (int)x0;
    return -1;
  }
  return x0;
#else
  // Linux ARM64
  register long x8 __asm__("x8") = number;
  register long x0 __asm__("x0") = arg1;
  register long x1 __asm__("x1") = arg2;
  register long x2 __asm__("x2") = arg3;
  register long x3 __asm__("x3") = arg4;

  __asm__ volatile("svc #0\n"
                   : "+r"(x0)
                   : "r"(x8), "r"(x1), "r"(x2), "r"(x3)
                   : "memory");
  if (x0 < 0 && x0 >= -4095) {
    errno = (int)-x0;
    return -1;
  }
  return x0;
#endif
}
#elif defined(__x86_64__)
/**
 * x86_64 Raw Syscall (Linux)
 */
static inline long raw_syscall(long number, long arg1, long arg2, long arg3,
                               long arg4) {
  long ret;
  __asm__ volatile("syscall"
                   : "=a"(ret)
                   : "a"(number), "D"(arg1), "S"(arg2), "d"(arg3), "r"(arg4)
                   : "rcx", "r11", "memory");
  if (ret < 0 && ret >= -4095) {
    errno = (int)-ret;
    return -1;
  }
  return ret;
}
#endif

/* --- Wrappers --- */

#if defined(__APPLE__)
int open_c_wrapper(const char *path, int flags, ...) {
#else
int open(const char *path, int flags, ...) {
#endif
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    mode = (mode_t)va_arg(ap, int);
    va_end(ap);
  }

  if (INITIALIZING) {
#if defined(__linux__) && defined(__aarch64__) && !defined(SYS_OPEN)
    return (int)raw_syscall(SYS_OPENAT, AT_FDCWD, (long)path, (long)flags,
                            (long)mode);
#else
    return (int)raw_syscall(SYS_OPEN, (long)path, (long)flags, (long)mode, 0);
#endif
  }
  return velo_open_impl(path, flags, mode);
}

#if defined(__APPLE__)
int openat_c_wrapper(int dirfd, const char *path, int flags, ...) {
#else
int openat(int dirfd, const char *path, int flags, ...) {
#endif
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    mode = (mode_t)va_arg(ap, int);
    va_end(ap);
  }

  if (INITIALIZING) {
    return (int)raw_syscall(SYS_OPENAT, (long)dirfd, (long)path, (long)flags,
                            (long)mode);
  }
  return velo_openat_impl(dirfd, path, flags, mode);
}

#if defined(__linux__)
// Linux specifics: open64 aliases
int open64(const char *path, int flags, ...) {
  va_list ap;
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_start(ap, flags);
    mode = va_arg(ap, int);
    va_end(ap);
  }
  return open(path, flags, mode);
}
int openat64(int dirfd, const char *path, int flags, ...) {
  va_list ap;
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_start(ap, flags);
    mode = va_arg(ap, int);
    va_end(ap);
  }
  return openat(dirfd, path, flags, mode);
}
#endif

#if defined(__APPLE__)
/* macOS Interpose - must be in the same file as the wrappers */
typedef struct interpose_s {
  void *new_func;
  void *old_func;
} interpose_t;

extern int open(const char *, int, ...);
extern int openat(int, const char *, int, ...);

__attribute__((used)) static const interpose_t interposers[]
    __attribute__((section("__DATA,__interpose"))) = {
        {(void *)open_c_wrapper, (void *)open},
        {(void *)openat_c_wrapper, (void *)openat},
    };
#endif
