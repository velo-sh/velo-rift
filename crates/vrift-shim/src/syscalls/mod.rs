// Syscall implementations
pub mod dir;
pub mod io;
#[cfg(target_os = "linux")]
pub mod linux_raw;
#[cfg(target_os = "macos")]
pub mod macos_raw;
pub mod mem;
pub mod misc;
pub mod mmap;
pub mod open;
pub mod path;
pub mod path_ops;
pub mod process;
pub mod stat;
pub mod vfs_ops;

// Re-export specific shims that need to be visible to interpose or extern C
#[cfg(target_os = "macos")]
pub use dir::*;
#[cfg(target_os = "macos")]
pub use misc::*;
#[cfg(target_os = "macos")]
pub use open::*;
#[cfg(target_os = "macos")]
pub use path::*;
#[cfg(target_os = "macos")]
pub use stat::*;
