//! Structured logging utilities for Velo Rift components.
//!
//! Provides consistent logging with component prefixes and structured fields.
//!
//! # Usage
//!
//! ```ignore
//! use vrift_config::logging::*;
//!
//! log_daemon_info!("Connection accepted", peer_pid = 1234);
//! log_cli_debug!("Sending request", request_type = "ingest");
//! ```

/// Component identifiers for log filtering
pub struct Component;

impl Component {
    pub const DAEMON: &'static str = "DAEMON";
    pub const CLI: &'static str = "CLI";
    pub const INCEPTION: &'static str = "INCEPTION";
    pub const INGEST: &'static str = "INGEST";
    pub const VFS: &'static str = "VFS";
    pub const IPC: &'static str = "IPC";
}

/// Log levels for runtime configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

// === DAEMON logging macros ===

#[macro_export]
macro_rules! log_daemon_error {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::error!(component = "DAEMON", $($key = $value,)* $msg)
    };
}

#[macro_export]
macro_rules! log_daemon_warn {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::warn!(component = "DAEMON", $($key = $value,)* $msg)
    };
}

#[macro_export]
macro_rules! log_daemon_info {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::info!(component = "DAEMON", $($key = $value,)* $msg)
    };
}

#[macro_export]
macro_rules! log_daemon_debug {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::debug!(component = "DAEMON", $($key = $value,)* $msg)
    };
}

// === CLI logging macros ===

#[macro_export]
macro_rules! log_cli_info {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::info!(component = "CLI", $($key = $value,)* $msg)
    };
}

#[macro_export]
macro_rules! log_cli_debug {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::debug!(component = "CLI", $($key = $value,)* $msg)
    };
}

// === INGEST logging macros ===

#[macro_export]
macro_rules! log_ingest_info {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::info!(component = "INGEST", $($key = $value,)* $msg)
    };
}

#[macro_export]
macro_rules! log_ingest_debug {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::debug!(component = "INGEST", $($key = $value,)* $msg)
    };
}

// === VFS logging macros ===

#[macro_export]
macro_rules! log_vfs_info {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::info!(component = "VFS", $($key = $value,)* $msg)
    };
}

#[macro_export]
macro_rules! log_vfs_debug {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::debug!(component = "VFS", $($key = $value,)* $msg)
    };
}

// === IPC logging macros ===

#[macro_export]
macro_rules! log_ipc_debug {
    ($msg:literal $(, $key:ident = $value:expr)* $(,)?) => {
        tracing::debug!(component = "IPC", $($key = $value,)* $msg)
    };
}

/// Initialize logging with the given level filter.
/// Call this once at application startup.
pub fn init_logging(level: LogLevel) {
    use tracing_subscriber::EnvFilter;

    let filter = match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    };

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_constants() {
        assert_eq!(Component::DAEMON, "DAEMON");
        assert_eq!(Component::CLI, "CLI");
        assert_eq!(Component::INCEPTION, "INCEPTION");
    }
}
