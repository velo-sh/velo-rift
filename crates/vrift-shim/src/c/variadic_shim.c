/**
 * C Variadic Shim for macOS ARM64
 *
 * Problem: Rust can't correctly handle variadic functions (open, openat).
 * Solution: C wrapper extracts variadic args, checks VFS_READY, then either:
 *   - If VFS not ready: direct syscall (zero Rust calls during dyld init)
 *   - If VFS ready: call Rust velo_*_impl for VFS logic
 *
 * Architecture:
 *   libc caller → C shim (va_list) → [VFS check] → Rust/syscall
 */

#include <fcntl.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

/* Rust VFS implementation functions - only called when VFS is ready */
extern int velo_open_impl(const char *path, int flags, mode_t mode);
extern int velo_openat_impl(int dirfd, const char *path, int flags,
                            mode_t mode);

/* VFS_READY flag exported from Rust - atomic bool (1 byte) */
extern _Atomic bool VFS_READY;

/**
 * open() variadic wrapper
 */
int open_c_wrapper(const char *path, int flags, ...) {
  mode_t mode = 0;

  if (flags & O_CREAT) {
    va_list ap;
    va_start(ap, flags);
    mode = (mode_t)va_arg(ap, int);
    va_end(ap);
  }

  /* Fast path: VFS not ready - direct syscall, no Rust calls */
  if (!VFS_READY) {
    return (int)syscall(SYS_open, path, flags, mode);
  }

  /* VFS path: call Rust implementation */
  return velo_open_impl(path, flags, mode);
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

  /* Fast path: VFS not ready - direct syscall, no Rust calls */
  if (!VFS_READY) {
#ifdef SYS_openat
    return (int)syscall(SYS_openat, dirfd, path, flags, mode);
#else
    extern int __openat(int, const char *, int, mode_t);
    return __openat(dirfd, path, flags, mode);
#endif
  }

  /* VFS path: call Rust implementation */
  return velo_openat_impl(dirfd, path, flags, mode);
}
