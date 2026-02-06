/**
 * Multi-Platform Variadic Shim Implementation
 *
 * Provides clean, fixed-argument entry points for Rust shims
 * to solve the Variadic ABI hazard on macOS ARM64.
 */

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <unistd.h>

/* RFC-0051: C-based errno bridge for cross-language consistency */
void set_vfs_errno(int e) { errno = e; }
int get_vfs_errno() { return errno; }

/* --- Platform Specific Syscall Numbers --- */

#if defined(__APPLE__) && defined(__aarch64__)
#define SYS_OPEN 5
#define SYS_OPENAT 463
#define SYS_STAT64 338
#define SYS_LSTAT64 340
#define SYS_ACCESS 33
#define SYS_READLINK 58
#define SYS_FSTAT64 339
#define SYS_FSTATAT64 466
#elif defined(__linux__) && defined(__x86_64__)
#define SYS_OPEN 2
#define SYS_OPENAT 257
#define SYS_STAT64 4
#define SYS_LSTAT64 6
#define SYS_ACCESS 21
#define SYS_READLINK 89
#elif defined(__linux__) && defined(__aarch64__)
#define SYS_OPENAT 56
#define SYS_STATAT 79
#define SYS_ACCESSAT 48
#define SYS_READLINKAT 78
#define SYS_FSTAT 80
#define SYS_FSTATAT 79
#define AT_FDCWD -100
#endif

/* --- External Rust Implementation & Flags --- */

extern int velo_open_impl(const char *path, int flags, mode_t mode);
extern int velo_openat_impl(int dirfd, const char *path, int flags,
                            mode_t mode);
extern int velo_stat_impl(const char *path, void *buf);
extern int velo_lstat_impl(const char *path, void *buf);
extern int velo_access_impl(const char *path, int mode);
extern long velo_readlink_impl(const char *path, char *buf, size_t bufsiz);
extern int velo_fstat_impl(int fd, void *buf);
extern int velo_fstatat_impl(int dirfd, const char *path, void *buf, int flags);

/* RFC-0049: Global initialization state
 * 2: Early-Init (Hazardous), 1: Rust-Init (Safe TLS), 0: Ready
 */
volatile char INITIALIZING = 2;

__attribute__((constructor(101))) void vfs_init_constructor() {
  // RFC-0051: Ignore SIGPIPE to prevent IPC failures from killing processes
  signal(SIGPIPE, SIG_IGN);
  INITIALIZING = 1;
}

// Late constructor to signal dyld bootstrap is complete
__attribute__((constructor(65535))) void vfs_late_init_constructor() {
  INITIALIZING = 0;
}

/* --- Raw Syscall Implementation --- */

#if defined(__aarch64__)
static inline long raw_syscall(long number, long arg1, long arg2, long arg3,
                               long arg4) {
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

/* --- Implementation Functions (called by Rust proxies or direct shims) --- */

// Linux interception is handled in interpose.rs using Rust shims to ensure
// reliable symbol export. macOS shimming uses this C bridge to handle variadic
// ABI.
#include <fcntl.h>
#include <stdarg.h>

#if defined(__APPLE__)
int c_open_bridge(const char *path, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_OPEN, (long)path, (long)flags, (long)mode, 0);
  }
  return velo_open_impl(path, flags, mode);
}

int c_openat_bridge(int dirfd, const char *path, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = (mode_t)va_arg(args, int);
    va_end(args);
  }
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_OPENAT, (long)dirfd, (long)path, (long)flags,
                            (long)mode);
  }
  return velo_openat_impl(dirfd, path, flags, mode);
}

int c_stat_bridge(const char *path, void *buf) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_STAT64, (long)path, (long)buf, 0, 0);
  }
  return velo_stat_impl(path, buf);
}

int c_lstat_bridge(const char *path, void *buf) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_LSTAT64, (long)path, (long)buf, 0, 0);
  }
  return velo_lstat_impl(path, buf);
}

int c_access_bridge(const char *path, int mode) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_ACCESS, (long)path, (long)mode, 0, 0);
  }
  return velo_access_impl(path, mode);
}

long c_readlink_bridge(const char *path, char *buf, size_t bufsiz) {
  if (INITIALIZING != 0) {
    return raw_syscall(SYS_READLINK, (long)path, (long)buf, (long)bufsiz, 0);
  }
  return velo_readlink_impl(path, buf, bufsiz);
}

int c_fstat_bridge(int fd, void *buf) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_FSTAT64, (long)fd, (long)buf, 0, 0);
  }
  return velo_fstat_impl(fd, buf);
}

int c_fstatat_bridge(int dirfd, const char *path, void *buf, int flags) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_FSTATAT64, (long)dirfd, (long)path, (long)buf,
                            (long)flags);
  }
  return velo_fstatat_impl(dirfd, path, buf, flags);
}
#endif

#define SYS_RENAME 128
#define SYS_RENAMEAT 444
#define SYS_FCNTL 92

extern int velo_rename_impl(const char *old, const char *new);
extern int velo_renameat_impl(int oldfd, const char *old, int newfd,
                              const char *new);

#if defined(__APPLE__)
int c_rename_bridge(const char *old, const char *new) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_RENAME, (long)old, (long)new, 0, 0);
  }
  return velo_rename_impl(old, new);
}

int c_renameat_bridge(int oldfd, const char *old, int newfd, const char *new) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_RENAMEAT, (long)oldfd, (long)old, (long)newfd,
                            (long)new);
  }
  return velo_renameat_impl(oldfd, old, newfd, new);
}

/* --- Metadata Hardening Bridges --- */

extern int creat_shim(const char *path, mode_t mode);
extern int getattrlist_shim(const char *path, void *attrlist, void *attrbuf,
                            size_t attrbufsize, unsigned long options);
extern int setattrlist_shim(const char *path, void *attrlist, void *attrbuf,
                            size_t attrbufsize, unsigned long options);

int c_creat_bridge(const char *path, mode_t mode) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_OPEN, (long)path,
                            (long)(O_CREAT | O_WRONLY | O_TRUNC), (long)mode,
                            0);
  }
  return creat_shim(path, mode);
}

int c_getattrlist_bridge(const char *path, void *attrlist, void *attrbuf,
                         size_t attrbufsize, unsigned long options) {
  return getattrlist_shim(path, attrlist, attrbuf, attrbufsize, options);
}

int c_setattrlist_bridge(const char *path, void *attrlist, void *attrbuf,
                         size_t attrbufsize, unsigned long options) {
  return setattrlist_shim(path, attrlist, attrbuf, attrbufsize, options);
}

/* --- fcntl variadic bridge --- */

extern int velo_fcntl_impl(int fd, int cmd, long arg);

#if defined(__APPLE__)
int fcntl_shim_c_impl(int fd, int cmd, long arg) {
  if (INITIALIZING != 0) {
    return (int)raw_syscall(SYS_FCNTL, (long)fd, (long)cmd, (long)arg, 0);
  }
  return velo_fcntl_impl(fd, cmd, arg);
}
#endif
#endif
