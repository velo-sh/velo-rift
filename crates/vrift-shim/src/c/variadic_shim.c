/**
 * C Variadic Shim for macOS ARM64 - Direct Syscall Design
 *
 * Problem: dlsym causes deadlock during dyld initialization.
 * Solution: Use direct syscall instruction, no dynamic resolution.
 *
 * macOS syscall numbers:
 *   open:   5
 *   openat: 463
 *
 * NOTE: This is a passthrough-only implementation. VFS logic requires
 * stable TLS/IPC which isn't available during early dyld init.
 */

#include <fcntl.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

/**
 * open() variadic wrapper - passes through to real syscall
 */
int open_c_wrapper(const char *path, int flags, ...) {
  mode_t mode = 0;

  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    mode = (mode_t)va_arg(ap, int);
    va_end(ap);
  }

  /* Direct syscall to avoid any interposition */
  return (int)syscall(SYS_open, path, flags, mode);
}

/**
 * openat() variadic wrapper
 */
int openat_c_wrapper(int dirfd, const char *path, int flags, ...) {
  mode_t mode = 0;

  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    mode = (mode_t)va_arg(ap, int);
    va_end(ap);
  }

#ifdef SYS_openat
  return (int)syscall(SYS_openat, dirfd, path, flags, mode);
#else
  /* Fallback: use __openat if available */
  extern int __openat(int, const char *, int, mode_t);
  return __openat(dirfd, path, flags, mode);
#endif
}
