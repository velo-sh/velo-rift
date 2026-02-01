// Syscall implementations
pub mod dir;
pub mod io;
pub mod mem;
pub mod misc;
pub mod mmap;
pub mod open;
pub mod path;
pub mod path_ops;
pub mod process;
pub mod stat;

// Re-export specific shims that need to be visible to interpose or extern C
#[cfg(target_os = "macos")]
pub use dir::*;
#[cfg(target_os = "macos")]
pub use misc::*;
#[cfg(target_os = "macos")]
pub use open::*;
#[cfg(target_os = "macos")]
pub use path::*;
pub use stat::*;
