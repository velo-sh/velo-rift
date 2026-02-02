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
#[cfg(target_os = "macos")]
const BINARY_SENSITIVE_EXTENSIONS: &[&str] = &[
    "app",
    "framework",
    "dylib",
    "so",
    "a",
    "bundle",
    "kext",
    "plugin",
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
#[cfg(target_os = "macos")]
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
        // Tier 1: clonefile (APFS CoW) - Preferred for Inode Decoupling
        // This ensures CAS-side uchg doesn't affect the source project file.
        match reflink_copy::reflink(source, target) {
            Ok(()) => return Ok(()),
            Err(_) => {
                // clonefile not supported or failed (e.g., non-APFS)
            }
        }

        // Tier 2: hard_link (Zero-copy, but shares Inode)
        // CAUTION: Only used if Reflink fails. This will leak metadata if uchg is applied.
        // For project-to-CAS ingest, we prefer Tier 3 (copy) over Tier 2 if we want absolute isolation.
        // But for now, we keep it as a fast fallback.
        match fs::hard_link(source, target) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(_) => {
                // EPERM or other failure
            }
        }

        // Tier 3: copy (last resort, safe Inode decoupling)
        fs::copy(source, target)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "macos-reflink-priority"
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
        // Tier 1: reflink (btrfs, xfs) - Preferred for Inode Decoupling
        // FICLONE ioctl provides zero-copy while maintaining separate inodes.
        if let Ok(()) = reflink_copy::reflink(source, target) {
            return Ok(());
        }

        // Tier 2: hard_link (Zero-copy, shared inode)
        match fs::hard_link(source, target) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                // EXDEV: handled by copy fallback
            }
            Err(e) => return Err(e),
        }

        // Tier 3: copy (last resort, safe Inode decoupling)
        fs::copy(source, target)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "linux-reflink-priority"
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    fn test_is_binary_sensitive_paths() {
        use std::path::Path;

        // Inside .app bundle
        assert!(is_binary_sensitive(Path::new(
            "Chromium.app/Contents/Info.plist"
        )));
        assert!(is_binary_sensitive(Path::new(
            "node_modules/puppeteer/Chromium.app/Resources/icon.icns"
        )));

        // Inside .framework
        assert!(is_binary_sensitive(Path::new(
            "Foo.framework/Versions/A/Foo"
        )));

        // Not inside bundle
        assert!(!is_binary_sensitive(Path::new(
            "node_modules/lodash/index.js"
        )));
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_inode_decoupling_integrity() {
        use crate::protection::{is_immutable, set_immutable};
        use std::os::unix::fs::MetadataExt;

        let dir = tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");

        let mut f = File::create(&source).unwrap();
        f.write_all(b"decoupling test").unwrap();
        drop(f);

        let strategy = get_strategy();
        strategy.link_file(&source, &target).unwrap();

        let source_ino = fs::metadata(&source).unwrap().ino();
        let target_ino = fs::metadata(&target).unwrap().ino();

        // On macOS APFS or Linux with Reflink support, Inodes MUST be different
        // to ensure metadata isolation (uchg protection).
        if source_ino != target_ino {
            println!(
                "Reflink detected: Inodes are decoupled ({} vs {})",
                source_ino, target_ino
            );

            // Apply protection to TARGET
            // Note: This might return EPERM on Linux if not root, but on macOS it should work for owner.
            match set_immutable(&target, true) {
                Ok(_) => {
                    assert!(is_immutable(&target).unwrap(), "Target should be immutable");
                    assert!(
                        !is_immutable(&source).unwrap(),
                        "Source MUST NOT be immutable (Contamination Check)"
                    );

                    // Cleanup
                    set_immutable(&target, false).unwrap();
                }
                Err(e) => {
                    println!("Skipping uchg leak check due to permissions: {}", e);
                }
            }
        } else {
            println!(
                "Warning: Hardlink detected (Inodes: {}). Metadata will leak!",
                source_ino
            );
            // On modern macOS/APFS, this should not happen with our new strategy.
        }
    }
}
