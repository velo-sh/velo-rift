//! # velo-shim
//!
//! LD_PRELOAD / DYLD_INSERT_LIBRARIES shim for Velo Rift virtual filesystem.
//! Industrial-grade, zero-allocation, and recursion-safe.

#![allow(clippy::missing_safety_doc)]
#![allow(unused_doc_comments)]
#![allow(dead_code)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::unnecessary_map_or)]

#[macro_use]
pub mod macros;
pub mod interpose;
pub mod ipc;
pub mod path;
pub mod state;
pub mod syscalls;

pub use syscalls::*;

#[allow(dead_code)]
extern "C" fn dump_logs_atexit() {} // Placeholder, moved to state.rs
