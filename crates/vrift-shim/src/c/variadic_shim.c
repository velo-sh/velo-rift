/**
 * Multi-Platform Variadic Shim Implementation
 *
 * Provides clean, fixed-argument entry points for Rust shims
 * to solve the Variadic ABI hazard on macOS ARM64.
 */

#include <errno.h>
#include <fcntl.h>
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
#elif defined(__linux__) && defined(__x86_64__)
#define SYS_OPEN 2
#define SYS_OPENAT 257
#elif defined(__linux__) && defined(__aarch64__)
#define SYS_OPENAT 56
#define AT_FDCWD -100
#endif

/* --- External Rust Implementation & Flags --- */

extern int velo_open_impl(const char *path, int flags, mode_t mode);
extern int velo_openat_impl(int dirfd, const char *path, int flags,
                            mode_t mode);

/* RFC-0049: Global initialization state
 * 2: Early-Init (Hazardous), 1: Rust-Init (Safe TLS), 0: Ready
 */
volatile char INITIALIZING = 2;

__attribute__((constructor)) void vfs_init_constructor() { INITIALIZING = 1; }

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
#if defined(__APPLE__)
int open_shim_c_impl(const char *path, int flags, mode_t mode) {
  // RFC-0050: State 2 (Early-Init) or 3 (Inside-Init) must use raw syscalls.
  // State 1 (Ready) transitions to Rust. State 0 (Done) is normal operation.
  if (INITIALIZING == 2 || INITIALIZING == 3) {
    return (int)raw_syscall(SYS_OPEN, (long)path, (long)flags, (long)mode, 0);
  }
  return velo_open_impl(path, flags, mode);
}

int openat_shim_c_impl(int dirfd, const char *path, int flags, mode_t mode) {
  if (INITIALIZING == 2 || INITIALIZING == 3) {
    return (int)raw_syscall(SYS_OPENAT, (long)dirfd, (long)path, (long)flags,
                            (long)mode);
  }
  return velo_openat_impl(dirfd, path, flags, mode);
}
#endif

/* --- fcntl variadic bridge --- */

extern int velo_fcntl_impl(int fd, int cmd, long arg);

#if defined(__APPLE__)
int fcntl_shim_c_impl(int fd, int cmd, long arg) {
  return velo_fcntl_impl(fd, cmd, arg);
}
#endif
