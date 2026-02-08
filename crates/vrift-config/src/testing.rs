//! Test environment abstraction for isolated testing.
//!
//! Provides `TestEnvironment` to manage:
//! - Isolated socket paths
//! - Temporary CAS roots
//! - Daemon lifecycle
//!
//! # Usage
//!
//! ```ignore
//! use vrift_config::testing::TestEnvironment;
//!
//! #[tokio::test]
//! async fn test_something() {
//!     let env = TestEnvironment::new().await;
//!     // env.socket_path, env.cas_root, env.manifest_path are all isolated
//!     // Daemon is NOT auto-started - tests control lifecycle
//! }
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tempfile::TempDir;

/// Atomic counter for unique test IDs
static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Isolated test environment with unique paths and optional daemon control
pub struct TestEnvironment {
    /// Temporary directory (dropped on cleanup)
    _temp_dir: TempDir,
    /// Unique socket path for this test
    pub socket_path: PathBuf,
    /// Isolated CAS root directory
    pub cas_root: PathBuf,
    /// Isolated manifest directory
    pub manifest_dir: PathBuf,
    /// Project root for the test
    pub project_root: PathBuf,
    /// Unique test ID
    pub test_id: u32,
}

impl TestEnvironment {
    /// Create a new isolated test environment
    pub fn new() -> anyhow::Result<Self> {
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create directory structure
        let project_root = root.join("project");
        let cas_root = root.join("cas");
        let manifest_dir = project_root.join(".vrift");

        std::fs::create_dir_all(&project_root)?;
        std::fs::create_dir_all(&cas_root)?;
        std::fs::create_dir_all(&manifest_dir)?;

        // Unique socket path
        let socket_path = root.join(format!("vrift-test-{}.sock", test_id));

        Ok(Self {
            _temp_dir: temp_dir,
            socket_path,
            cas_root,
            manifest_dir,
            project_root,
            test_id,
        })
    }

    /// Get manifest LMDB path
    pub fn manifest_path(&self) -> PathBuf {
        self.manifest_dir.join("manifest.lmdb")
    }

    /// Get staging directory path
    pub fn staging_dir(&self) -> PathBuf {
        self.project_root.join(".vrift").join("staging")
    }

    /// Create a test file with content
    pub fn create_file(&self, relative_path: &str, content: &[u8]) -> anyhow::Result<PathBuf> {
        let path = self.project_root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(path)
    }

    /// Create a test directory
    pub fn create_dir(&self, relative_path: &str) -> anyhow::Result<PathBuf> {
        let path = self.project_root.join(relative_path);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Check if socket exists (daemon may be running)
    pub fn is_socket_present(&self) -> bool {
        self.socket_path.exists()
    }

    /// Get environment variables for spawning daemon/shim with this config.
    ///
    /// Builds a Config from test paths and uses `shim_env()` for consistency
    /// with production code, plus daemon-specific vars.
    pub fn daemon_env(&self) -> Vec<(String, String)> {
        let mut cfg = crate::Config::default();
        cfg.storage.the_source = self.cas_root.clone();
        cfg.daemon.socket = self.socket_path.clone();
        cfg.project.root = self.project_root.clone();
        cfg.project.manifest = self.manifest_path();

        
        // Daemon also needs VRIFT_SOCKET_PATH explicitly (shim_env includes it)
        // Add any daemon-only vars here if needed in the future
        cfg.shim_env()
    }
}

impl Default for TestEnvironment {
    fn default() -> Self {
        Self::new().expect("Failed to create test environment")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_creates_directories() {
        let env = TestEnvironment::new().unwrap();
        assert!(env.project_root.exists());
        assert!(env.cas_root.exists());
        assert!(env.manifest_dir.exists());
    }

    #[test]
    fn test_environment_has_unique_socket() {
        let env1 = TestEnvironment::new().unwrap();
        let env2 = TestEnvironment::new().unwrap();
        assert_ne!(env1.socket_path, env2.socket_path);
    }

    #[test]
    fn test_create_file() {
        let env = TestEnvironment::new().unwrap();
        let path = env.create_file("src/main.rs", b"fn main() {}").unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"fn main() {}");
    }

    #[test]
    fn test_daemon_env() {
        let env = TestEnvironment::new().unwrap();
        let vars = env.daemon_env();
        assert!(vars.iter().any(|(k, _)| k == "VRIFT_SOCKET_PATH"));
        assert!(vars.iter().any(|(k, _)| k == "VR_THE_SOURCE"));
    }
}
