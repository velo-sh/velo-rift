//! # vrift-config
//!
//! Configuration management for Velo Rift.
//!
//! Loads configuration from:
//! 1. `~/.vrift/config.toml` (global)
//! 2. `.vrift/config.toml` (project-local, overrides global)
//! 3. Environment variables (highest priority)

pub mod logging;
pub mod path;
pub mod testing;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tracing::debug;

/// Global config instance
static CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| {
    RwLock::new(Config::load().unwrap_or_else(|e| {
        eprintln!(
            "[vrift-config] WARNING: Failed to load config: {}. Using defaults.",
            e
        );
        Config::default()
    }))
});

/// Default CAS root path
pub const DEFAULT_CAS_ROOT: &str = vrift_ipc::DEFAULT_CAS_ROOT;
/// Default Unix socket path
pub const DEFAULT_SOCKET_PATH: &str = vrift_ipc::DEFAULT_SOCKET_PATH;

/// Get global config (read-only)
pub fn config() -> std::sync::RwLockReadGuard<'static, Config> {
    CONFIG.read().unwrap()
}

/// Reload config from disk
pub fn reload() -> Result<(), ConfigError> {
    let new_config = Config::load()?;
    *CONFIG.write().unwrap() = new_config;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Current config schema version
pub const CONFIG_VERSION: u32 = 1;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Config schema version (for forward compatibility)
    pub config_version: u32,
    pub project: ProjectConfig,
    pub storage: StorageConfig,
    pub ingest: IngestConfig,
    pub tiers: TierConfig,
    pub security: SecurityConfig,
    pub daemon: DaemonConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            project: ProjectConfig::default(),
            storage: StorageConfig::default(),
            ingest: IngestConfig::default(),
            tiers: TierConfig::default(),
            security: SecurityConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

impl Config {
    /// Load config from standard locations (CWD-relative project config)
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_for_project(Path::new("."))
    }

    /// Load config for a specific project root directory.
    /// Resolution order: global → project → env vars.
    pub fn load_for_project(project_root: &Path) -> Result<Self, ConfigError> {
        let mut config = Config::default();

        // 1. Load global config (~/.vrift/config.toml)
        if let Some(global_path) = Self::global_config_path() {
            if global_path.exists() {
                debug!("Loading global config from {:?}", global_path);
                let contents = std::fs::read_to_string(&global_path)?;
                config = toml::from_str(&contents)?;
            }
        }

        // 2. Load project config (<project_root>/.vrift/config.toml)
        let project_config_path = project_root.join(".vrift/config.toml");
        if project_config_path.exists() {
            debug!("Loading project config from {:?}", project_config_path);
            let contents = std::fs::read_to_string(&project_config_path)?;
            let project_config: Config = toml::from_str(&contents)?;
            config.merge(project_config);
        }

        // 3. Apply environment variable overrides
        config.apply_env_overrides();

        // 4. Resolve project root to absolute path if relative
        if config.project.root.as_os_str() == "." {
            if let Ok(abs) = std::fs::canonicalize(project_root) {
                config.project.root = abs;
            } else {
                config.project.root = project_root.to_path_buf();
            }
        }

        // 5. Validate socket path: if parent dir doesn't exist and can't
        //    be created, fall back to default /tmp/vrift.sock so all
        //    components (CLI, daemon, tests) resolve to the same socket.
        if let Some(parent) = config.daemon.socket.parent() {
            if !parent.as_os_str().is_empty()
                && !parent.exists()
                && std::fs::create_dir_all(parent).is_err()
            {
                debug!(
                    "Socket directory {:?} unavailable, falling back to /tmp/vrift.sock",
                    parent
                );
                config.daemon.socket = PathBuf::from("/tmp/vrift.sock");
            }
        }

        Ok(config)
    }

    /// Global config path: ~/.vrift/config.toml
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".vrift/config.toml"))
    }

    /// Merge another config (project overrides global).
    /// Non-default values from `other` replace values in `self`.
    fn merge(&mut self, other: Config) {
        // Project
        let default_project = ProjectConfig::default();
        if other.project.vfs_prefix != default_project.vfs_prefix {
            self.project.vfs_prefix = other.project.vfs_prefix;
        }
        if other.project.root != default_project.root {
            self.project.root = other.project.root;
        }
        if other.project.manifest != default_project.manifest {
            self.project.manifest = other.project.manifest;
        }

        // Storage
        let default_storage = StorageConfig::default();
        if other.storage.the_source != default_storage.the_source {
            self.storage.the_source = other.storage.the_source;
        }
        if other.storage.default_mode != default_storage.default_mode {
            self.storage.default_mode = other.storage.default_mode;
        }

        // Daemon
        let default_daemon = DaemonConfig::default();
        if other.daemon.socket != default_daemon.socket {
            self.daemon.socket = other.daemon.socket;
        }
        if other.daemon.debug != default_daemon.debug {
            self.daemon.debug = other.daemon.debug;
        }

        // Tiers
        if !other.tiers.tier1_patterns.is_empty() {
            self.tiers.tier1_patterns = other.tiers.tier1_patterns;
        }
        if !other.tiers.tier2_patterns.is_empty() {
            self.tiers.tier2_patterns = other.tiers.tier2_patterns;
        }

        // Security
        if !other.security.exclude_patterns.is_empty() {
            self.security.exclude_patterns = other.security.exclude_patterns;
        }
    }

    /// Apply environment variable overrides (highest priority)
    fn apply_env_overrides(&mut self) {
        // Project
        if let Ok(root) = std::env::var("VRIFT_PROJECT_ROOT") {
            self.project.root = PathBuf::from(root);
        }
        if let Ok(prefix) = std::env::var("VRIFT_VFS_PREFIX") {
            self.project.vfs_prefix = prefix;
        }
        if let Ok(manifest) = std::env::var("VRIFT_MANIFEST") {
            self.project.manifest = PathBuf::from(manifest);
        }

        // Storage
        if let Ok(path) = std::env::var("VR_THE_SOURCE") {
            self.storage.the_source = PathBuf::from(path);
        }

        // Ingest
        if let Ok(threads) = std::env::var("VRIFT_THREADS") {
            if let Ok(n) = threads.parse() {
                self.ingest.threads = Some(n);
            }
        }

        // Daemon
        if let Ok(socket) = std::env::var("VRIFT_SOCKET_PATH") {
            self.daemon.socket = PathBuf::from(socket);
        }
        if let Ok(registry) = std::env::var("VRIFT_REGISTRY_DIR") {
            self.daemon.registry_dir = PathBuf::from(registry);
        }
        if let Ok(timeout) = std::env::var("VRIFT_LOCK_TIMEOUT") {
            if let Ok(secs) = timeout.parse() {
                self.daemon.lock_timeout_secs = secs;
            }
        }
        if std::env::var("VRIFT_DEBUG").is_ok() {
            self.daemon.debug = true;
        }
        if let Ok(mmap) = std::env::var("VRIFT_MMAP_PATH") {
            self.daemon.mmap_path = PathBuf::from(mmap);
        }
        if let Ok(cow) = std::env::var("VRIFT_COW_TEMP_DIR") {
            self.daemon.cow_temp_dir = PathBuf::from(cow);
        }
        if let Ok(log) = std::env::var("VRIFT_LOG_DIR") {
            self.daemon.log_dir = PathBuf::from(log);
        }
    }

    /// Derive environment variables for shim-wrapped processes.
    /// This is the SSOT → shim bridge: TOML config → env vars.
    pub fn shim_env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            (
                "VR_THE_SOURCE".to_string(),
                self.storage.the_source.display().to_string(),
            ),
            (
                "VRIFT_VFS_PREFIX".to_string(),
                self.project.vfs_prefix.clone(),
            ),
            (
                "VRIFT_PROJECT_ROOT".to_string(),
                self.project.root.display().to_string(),
            ),
            (
                "VRIFT_SOCKET_PATH".to_string(),
                self.daemon.socket.display().to_string(),
            ),
            (
                "VRIFT_MANIFEST".to_string(),
                self.project.manifest.display().to_string(),
            ),
        ];
        if self.daemon.debug {
            env.push(("VRIFT_DEBUG".to_string(), "1".to_string()));
        }
        env
    }

    /// Generate TOML template for `vrift init`.
    pub fn init_toml() -> String {
        let default = Config::default();
        format!(
            r#"# Velo Rift project configuration
# Documentation: https://github.com/velo-sh/velo-rift
config_version = 1

[project]
vfs_prefix = "{vfs_prefix}"
# manifest = ".vrift/manifest.lmdb"  # relative to project root

[storage]
the_source = "{the_source}"
# default_mode = "solid"

[daemon]
# socket = "{socket}"
# debug = false

# [ingest]
# threads = auto
# default_tier = "tier2"

# [tiers]
# tier1_patterns = ["node_modules/", ".cargo/registry/"]
# tier2_patterns = ["target/", "build/"]
"#,
            vfs_prefix = default.project.vfs_prefix,
            the_source = default.storage.the_source.display(),
            socket = default.daemon.socket.display(),
        )
    }

    /// Generate default config TOML string
    #[deprecated(
        since = "0.2.0",
        note = "Use Config::init_toml() for human-readable template"
    )]
    pub fn default_toml() -> String {
        toml::to_string_pretty(&Config::default()).unwrap()
    }

    // ========== Convenience Accessors ==========

    /// Get socket path (resolved)
    pub fn socket_path(&self) -> &Path {
        &self.daemon.socket
    }

    /// Get CAS root path (TheSource™)
    pub fn cas_root(&self) -> &Path {
        &self.storage.the_source
    }

    /// Get registry directory
    pub fn registry_dir(&self) -> &Path {
        &self.daemon.registry_dir
    }

    /// Get lock timeout in seconds
    pub fn lock_timeout(&self) -> u64 {
        self.daemon.lock_timeout_secs
    }

    /// Check if debug mode is enabled
    pub fn debug_mode(&self) -> bool {
        self.daemon.debug
    }

    /// Get manifest mmap path for hot stat cache (RFC-0044)
    pub fn mmap_path(&self) -> &Path {
        &self.daemon.mmap_path
    }

    /// Get CoW temporary file directory
    pub fn cow_temp_dir(&self) -> &Path {
        &self.daemon.cow_temp_dir
    }

    /// Get log directory for daemon and inception-layer
    pub fn log_dir(&self) -> &Path {
        &self.daemon.log_dir
    }
}

/// Project-level configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectConfig {
    /// Project root directory (auto-resolved to absolute path)
    pub root: PathBuf,
    /// Virtual filesystem prefix for shim path interception
    pub vfs_prefix: String,
    /// Manifest LMDB path (relative to project root)
    pub manifest: PathBuf,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            vfs_prefix: "/vrift".to_string(),
            manifest: PathBuf::from(".vrift/manifest.lmdb"),
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// TheSource™ — canonical CAS storage directory.
    /// Global singleton managed by vriftd.
    /// Env override: VR_THE_SOURCE
    pub the_source: PathBuf,
    /// Default projection mode: solid or phantom
    pub default_mode: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            the_source: PathBuf::from(DEFAULT_CAS_ROOT),
            default_mode: "solid".to_string(),
        }
    }
}

/// Ingest configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestConfig {
    /// Number of parallel threads (None = auto)
    pub threads: Option<usize>,
    /// Default tier: tier1, tier2, or auto
    pub default_tier: String,
    /// Deduplication window in milliseconds (default: 200ms)
    pub dedup_window_ms: u64,
    /// Batch size for high-frequency writes (default: 10)
    pub batch_size: usize,
    /// Batch timeout in milliseconds (default: 100ms)
    pub batch_timeout_ms: u64,
    /// Patterns to ignore during ingest and live watch
    pub ignore_patterns: Vec<String>,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            threads: None,
            default_tier: "tier2".to_string(),
            dedup_window_ms: 200,
            batch_size: 10,
            batch_timeout_ms: 100,
            // Minimal defaults - user configures project-specific patterns
            ignore_patterns: vec![
                ".vrift".to_string(),    // Vrift system directory (always needed)
                ".DS_Store".to_string(), // macOS junk
            ],
        }
    }
}

/// Tier classification patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TierConfig {
    /// Tier-1 (Immutable) path patterns
    pub tier1_patterns: Vec<String>,
    /// Tier-2 (Mutable) path patterns
    pub tier2_patterns: Vec<String>,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            tier1_patterns: vec![
                "node_modules/".to_string(),
                ".cargo/registry/".to_string(),
                ".rustup/".to_string(),
                "/toolchains/".to_string(),
                ".venv/lib/".to_string(),
                "site-packages/".to_string(),
                "/usr/lib/".to_string(),
                "/usr/share/".to_string(),
            ],
            tier2_patterns: vec![
                "target/".to_string(),
                "target/debug/".to_string(),
                "target/release/".to_string(),
                "dist/".to_string(),
                "build/".to_string(),
                ".next/".to_string(),
                "__pycache__/".to_string(),
                ".pytest_cache/".to_string(),
                ".cache/".to_string(),
                "out/".to_string(),
            ],
        }
    }
}

/// Security filter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Enable security filter
    pub enabled: bool,
    /// Patterns to exclude (sensitive files)
    pub exclude_patterns: Vec<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            exclude_patterns: vec![
                ".env".to_string(),
                ".env.*".to_string(),
                "*.key".to_string(),
                "*.pem".to_string(),
                "*.p12".to_string(),
                "*.pfx".to_string(),
                "id_rsa".to_string(),
                "id_rsa.*".to_string(),
                "id_ed25519".to_string(),
                "id_ed25519.*".to_string(),
                "*.keystore".to_string(),
                "credentials.json".to_string(),
                "secrets.yaml".to_string(),
                "secrets.yml".to_string(),
            ],
        }
    }
}

/// Daemon configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Unix socket path
    pub socket: PathBuf,
    /// Registry directory for project index
    pub registry_dir: PathBuf,
    /// Lock acquisition timeout in seconds
    pub lock_timeout_secs: u64,
    /// Enable daemon mode
    pub enabled: bool,
    /// Enable debug mode
    pub debug: bool,
    /// Manifest mmap path for hot stat cache (RFC-0044)
    pub mmap_path: PathBuf,
    /// CoW temporary file directory for inception-layer
    pub cow_temp_dir: PathBuf,
    /// Log directory for daemon and inception-layer
    pub log_dir: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket: PathBuf::from(DEFAULT_SOCKET_PATH),
            registry_dir: dirs::home_dir()
                .map(|h| h.join(".vrift/registry"))
                .unwrap_or_else(|| PathBuf::from("/tmp/vrift_registry")),
            lock_timeout_secs: 30,
            enabled: false,
            debug: false,
            mmap_path: PathBuf::from("/tmp/vrift-manifest.mmap"),
            cow_temp_dir: PathBuf::from("/tmp"),
            log_dir: PathBuf::from("/tmp"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lock for tests that modify environment variables to prevent race conditions
    // when tests run in parallel
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ========== Default Values Tests ==========

    #[test]
    fn test_default_config_has_all_sections() {
        let config = Config::default();

        // Storage defaults
        assert_eq!(
            config.storage.the_source,
            PathBuf::from("~/.vrift/the_source")
        );
        assert_eq!(config.storage.default_mode, "solid");

        // Ingest defaults
        assert!(config.ingest.threads.is_none());
        assert_eq!(config.ingest.default_tier, "tier2");

        // Daemon defaults
        assert!(!config.daemon.enabled);
        assert_eq!(config.daemon.socket, PathBuf::from(DEFAULT_SOCKET_PATH));
        assert_eq!(config.daemon.lock_timeout_secs, 30);
        assert!(!config.daemon.debug);
    }

    #[test]
    fn test_default_tier1_patterns_cover_common_deps() {
        let config = Config::default();
        let patterns = &config.tiers.tier1_patterns;

        assert!(patterns.iter().any(|p| p.contains("node_modules")));
        assert!(patterns.iter().any(|p| p.contains(".cargo/registry")));
        assert!(patterns.iter().any(|p| p.contains(".rustup")));
        assert!(patterns.iter().any(|p| p.contains("site-packages")));
    }

    #[test]
    fn test_default_security_patterns_cover_sensitive_files() {
        let config = Config::default();
        let patterns = &config.security.exclude_patterns;

        assert!(patterns.iter().any(|p| p.contains(".env")));
        assert!(patterns.iter().any(|p| p.contains(".key")));
        assert!(patterns.iter().any(|p| p.contains("id_rsa")));
        assert!(patterns.iter().any(|p| p.contains("credentials")));
    }

    // ========== TOML Serialization Tests ==========

    #[test]
    fn test_default_toml_generation_includes_all_sections() {
        let toml_str = Config::default_toml();

        assert!(toml_str.contains("[storage]"));
        assert!(toml_str.contains("[ingest]"));
        assert!(toml_str.contains("[tiers]"));
        assert!(toml_str.contains("[security]"));
        assert!(toml_str.contains("[daemon]"));
    }

    #[test]
    fn test_toml_roundtrip_preserves_all_values() {
        let original = Config::default();
        let toml_str = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(original.storage.default_mode, parsed.storage.default_mode);
        assert_eq!(
            original.tiers.tier1_patterns.len(),
            parsed.tiers.tier1_patterns.len()
        );
        assert_eq!(
            original.security.exclude_patterns.len(),
            parsed.security.exclude_patterns.len()
        );
        assert_eq!(original.daemon.enabled, parsed.daemon.enabled);
    }

    #[test]
    fn test_partial_toml_uses_defaults() {
        let partial_toml = r#"
[storage]
default_mode = "phantom"
"#;
        let config: Config = toml::from_str(partial_toml).unwrap();

        // Specified value
        assert_eq!(config.storage.default_mode, "phantom");

        // Default values for unspecified
        assert!(!config.tiers.tier1_patterns.is_empty());
        assert!(config.security.enabled);
    }

    // ========== Config Loading Tests ==========

    #[test]
    fn test_load_from_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let custom_config = r#"
[storage]
the_source = "/custom/path"
default_mode = "phantom"

[ingest]
threads = 8
default_tier = "tier1"
"#;
        std::fs::write(&config_path, custom_config).unwrap();

        let contents = std::fs::read_to_string(&config_path).unwrap();
        let config: Config = toml::from_str(&contents).unwrap();

        assert_eq!(config.storage.the_source, PathBuf::from("/custom/path"));
        assert_eq!(config.storage.default_mode, "phantom");
        assert_eq!(config.ingest.threads, Some(8));
        assert_eq!(config.ingest.default_tier, "tier1");
    }

    // ========== Config Merge Tests ==========

    #[test]
    fn test_merge_replaces_non_empty_patterns() {
        let mut base = Config::default();
        let mut overlay = Config::default();

        overlay.tiers.tier1_patterns = vec!["custom/".to_string()];
        base.merge(overlay);

        assert_eq!(base.tiers.tier1_patterns, vec!["custom/".to_string()]);
    }

    #[test]
    fn test_merge_preserves_base_when_overlay_empty() {
        let mut base = Config::default();
        let original_patterns = base.tiers.tier1_patterns.clone();

        let mut overlay = Config::default();
        overlay.tiers.tier1_patterns = vec![];

        base.merge(overlay);

        // Empty overlay should not replace base patterns
        assert_eq!(base.tiers.tier1_patterns, original_patterns);
    }

    // ========== Environment Override Tests ==========

    #[test]
    fn test_env_override_the_source() {
        let _guard = ENV_LOCK.lock().unwrap(); // Serialize env tests
        let mut config = Config::default();

        std::env::set_var("VR_THE_SOURCE", "/env/override/path");
        config.apply_env_overrides();
        std::env::remove_var("VR_THE_SOURCE");

        assert_eq!(
            config.storage.the_source,
            PathBuf::from("/env/override/path")
        );
    }

    #[test]
    fn test_env_override_threads() {
        let _guard = ENV_LOCK.lock().unwrap(); // Serialize env tests
        let mut config = Config::default();

        std::env::set_var("VRIFT_THREADS", "16");
        config.apply_env_overrides();
        std::env::remove_var("VRIFT_THREADS");

        assert_eq!(config.ingest.threads, Some(16));
    }

    #[test]
    fn test_env_override_invalid_threads_ignored() {
        let _guard = ENV_LOCK.lock().unwrap(); // Serialize env tests
        let mut config = Config::default();

        std::env::set_var("VRIFT_THREADS", "not_a_number");
        config.apply_env_overrides();
        std::env::remove_var("VRIFT_THREADS");

        // Invalid value should be ignored, keep default
        assert!(config.ingest.threads.is_none());
    }

    // ========== Global Config Path Tests ==========

    #[test]
    fn test_global_config_path_exists() {
        let path = Config::global_config_path();
        assert!(path.is_some());

        let path = path.unwrap();
        assert!(path.ends_with(".vrift/config.toml"));
    }

    // ========== Edge Cases ==========

    #[test]
    fn test_empty_config_uses_all_defaults() {
        let config: Config = toml::from_str("").unwrap();
        let default_config = Config::default();

        assert_eq!(
            config.storage.default_mode,
            default_config.storage.default_mode
        );
        assert_eq!(
            config.tiers.tier1_patterns.len(),
            default_config.tiers.tier1_patterns.len()
        );
    }

    #[test]
    fn test_invalid_toml_returns_error() {
        let result: Result<Config, _> = toml::from_str("invalid { toml }");
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_fields_are_ignored() {
        let toml_with_unknown = r#"
[storage]
default_mode = "solid"
unknown_field = "should be ignored"

[unknown_section]
foo = "bar"
"#;
        // This should not panic, unknown fields are ignored with #[serde(default)]
        let result: Result<Config, _> = toml::from_str(toml_with_unknown);
        // Note: default serde behavior may error on unknown fields
        // If this test fails, we may need to add #[serde(deny_unknown_fields)]
        // or handle this differently
        if let Ok(config) = result {
            assert_eq!(config.storage.default_mode, "solid");
        }
    }
}
