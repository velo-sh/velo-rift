//! Cross-Platform Link Strategy (RFC-0040)
//!
//! Provides platform-optimized file linking for CAS storage.
//!
//! # Design
//!
//! - **Linux**: Prefer hard_link, future io_uring batching
//! - **macOS**: Tiered fallback (hard_link → clonefile → copy)
//!
//! # Usage
//!
//! ```ignore
//! use vrift_cas::link_strategy::get_strategy;
//!
//! let strategy = get_strategy();
//! strategy.link_file(source, target)?;
//! ```

use std::fs;
use std::io;
use std::path::Path;

// ============================================================================
// LinkStrategy Trait
// ============================================================================

/// Platform-agnostic file linking strategy for CAS storage
pub trait LinkStrategy: Send + Sync {
    /// Link or copy a file from source to target
    ///
    /// Implementations should choose the most efficient method:
    /// - hard_link (zero-copy, shared inode)
    /// - clonefile (zero-copy, separate inode)  
    /// - copy (fallback)
    fn link_file(&self, source: &Path, target: &Path) -> io::Result<()>;
    
    /// Name of this strategy (for logging/debugging)
    fn name(&self) -> &'static str;
}

// ============================================================================
// Binary-Sensitive Path Detection
// ============================================================================

/// File extensions that should skip hard_link on macOS
///
/// These typically involve:
/// - Code-signed bundles (.app, .framework)
/// - Dynamic libraries (.dylib, .so)
/// - Static archives (.a)
/// - Kernel extensions (.kext, .bundle)
const BINARY_SENSITIVE_EXTENSIONS: &[&str] = &[
    "app", "framework", "dylib", "so", "a", "bundle", "kext", "plugin",
];

/// Check if a path is inside a binary-sensitive bundle
///
/// Returns true for:
/// - Files with sensitive extensions
/// - Files inside .app/ or .framework/ directories
///
/// # Example
/// ```ignore
/// assert!(is_binary_sensitive(Path::new("Chromium.app/Contents/Info.plist")));
/// assert!(is_binary_sensitive(Path::new("libfoo.dylib")));
/// assert!(!is_binary_sensitive(Path::new("index.js")));
/// ```
pub fn is_binary_sensitive(path: &Path) -> bool {
    // Check extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if BINARY_SENSITIVE_EXTENSIONS.contains(&ext) {
            return true;
        }
    }
    
    // Check if inside .app/ or .framework/ directory
    let path_str = path.to_string_lossy();
    path_str.contains(".app/") || path_str.contains(".framework/")
}

/// Direct reflink or copy (skipping hard_link attempt)
fn reflink_or_copy(source: &Path, target: &Path) -> io::Result<()> {
    match reflink_copy::reflink(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, target)?;
            Ok(())
        }
    }
}

// ============================================================================
// macOS Implementation
// ============================================================================

/// macOS link strategy: hard_link → clonefile → copy
///
/// Uses tiered fallback to handle code-signed bundles (.app, .framework)
/// that reject hard_link with EPERM.
///
/// **Fast-path**: Known binary-sensitive paths skip hard_link entirely.
#[cfg(target_os = "macos")]
pub struct MacosLinkStrategy;

#[cfg(target_os = "macos")]
impl LinkStrategy for MacosLinkStrategy {
    fn link_file(&self, source: &Path, target: &Path) -> io::Result<()> {
        // Fast-path: skip hard_link for known-sensitive paths
        if is_binary_sensitive(source) {
            return reflink_or_copy(source, target);
        }
        
        // Tier 1: hard_link (most efficient)
        match fs::hard_link(source, target) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                // EPERM: likely code-signed bundle, try clonefile
            }
            Err(e) => return Err(e),
        }
        
        // Tier 2: clonefile (APFS CoW)
        match reflink_copy::reflink(source, target) {
            Ok(()) => return Ok(()),
            Err(_) => {
                // clonefile not supported or failed
            }
        }
        
        // Tier 3: copy (last resort)
        fs::copy(source, target)?;
        Ok(())
    }
    
    fn name(&self) -> &'static str {
        "macos-tiered"
    }
}

// ============================================================================
// Linux Implementation
// ============================================================================

/// Linux link strategy: hard_link (with reflink fallback)
///
/// Prefers hard_link for maximum efficiency. Falls back to reflink
/// on filesystems that support it (btrfs, xfs), then copy.
#[cfg(target_os = "linux")]
pub struct LinuxLinkStrategy;

#[cfg(target_os = "linux")]
impl LinkStrategy for LinuxLinkStrategy {
    fn link_file(&self, source: &Path, target: &Path) -> io::Result<()> {
        // Tier 1: hard_link (preferred on Linux)
        match fs::hard_link(source, target) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                // EXDEV: cross-device link, try reflink
            }
            Err(e) => return Err(e),
        }
        
        // Tier 2: reflink (btrfs, xfs)
        if let Ok(()) = reflink_copy::reflink(source, target) {
            return Ok(());
        }
        
        // Tier 3: copy
        fs::copy(source, target)?;
        Ok(())
    }
    
    fn name(&self) -> &'static str {
        "linux-hardlink"
    }
}

// ============================================================================
// Fallback Implementation (other Unix)
// ============================================================================

/// Generic Unix fallback: hard_link → copy
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct UnixLinkStrategy;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl LinkStrategy for UnixLinkStrategy {
    fn link_file(&self, source: &Path, target: &Path) -> io::Result<()> {
        match fs::hard_link(source, target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(_) => {
                fs::copy(source, target)?;
                Ok(())
            }
        }
    }
    
    fn name(&self) -> &'static str {
        "unix-generic"
    }
}

// ============================================================================
// Factory Function
// ============================================================================

/// Get the platform-optimal LinkStrategy
#[cfg(target_os = "macos")]
pub fn get_strategy() -> &'static dyn LinkStrategy {
    static STRATEGY: MacosLinkStrategy = MacosLinkStrategy;
    &STRATEGY
}

#[cfg(target_os = "linux")]
pub fn get_strategy() -> &'static dyn LinkStrategy {
    static STRATEGY: LinuxLinkStrategy = LinuxLinkStrategy;
    &STRATEGY
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn get_strategy() -> &'static dyn LinkStrategy {
    static STRATEGY: UnixLinkStrategy = UnixLinkStrategy;
    &STRATEGY
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;
    
    #[test]
    fn test_strategy_name() {
        let strategy = get_strategy();
        assert!(!strategy.name().is_empty());
    }
    
    #[test]
    fn test_link_file_success() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        
        let mut f = File::create(&source).unwrap();
        f.write_all(b"hello").unwrap();
        
        let strategy = get_strategy();
        strategy.link_file(&source, &target).unwrap();
        
        assert!(target.exists());
    }
    
    #[test]
    fn test_link_file_already_exists() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        
        File::create(&source).unwrap();
        File::create(&target).unwrap();
        
        let strategy = get_strategy();
        // Should not error on AlreadyExists
        let result = strategy.link_file(&source, &target);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_is_binary_sensitive_extensions() {
        use std::path::Path;
        
        // Sensitive extensions
        assert!(is_binary_sensitive(Path::new("Chromium.app")));
        assert!(is_binary_sensitive(Path::new("libfoo.dylib")));
        assert!(is_binary_sensitive(Path::new("libbar.so")));
        assert!(is_binary_sensitive(Path::new("libstatic.a")));
        assert!(is_binary_sensitive(Path::new("MyPlugin.bundle")));
        assert!(is_binary_sensitive(Path::new("Cocoa.framework")));
        
        // Non-sensitive
        assert!(!is_binary_sensitive(Path::new("index.js")));
        assert!(!is_binary_sensitive(Path::new("package.json")));
        assert!(!is_binary_sensitive(Path::new("README.md")));
    }
    
    #[test]
    fn test_is_binary_sensitive_paths() {
        use std::path::Path;
        
        // Inside .app bundle
        assert!(is_binary_sensitive(Path::new("Chromium.app/Contents/Info.plist")));
        assert!(is_binary_sensitive(Path::new("node_modules/puppeteer/Chromium.app/Resources/icon.icns")));
        
        // Inside .framework
        assert!(is_binary_sensitive(Path::new("Foo.framework/Versions/A/Foo")));
        
        // Not inside bundle
        assert!(!is_binary_sensitive(Path::new("node_modules/lodash/index.js")));
    }
}
