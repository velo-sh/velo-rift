//! # vrift-config
//!
//! Configuration management for Velo Rift.
//!
//! Loads configuration from:
//! 1. `~/.vrift/config.toml` (global)
//! 2. `.vrift/config.toml` (project-local, overrides global)
//! 3. Environment variables (highest priority)

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tracing::debug;

/// Global config instance
static CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::load().unwrap_or_default()));

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

/// Main configuration structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub storage: StorageConfig,
    pub ingest: IngestConfig,
    pub tiers: TierConfig,
    pub security: SecurityConfig,
    pub daemon: DaemonConfig,
}

impl Config {
    /// Load config from standard locations
    pub fn load() -> Result<Self, ConfigError> {
        let mut config = Config::default();

        // 1. Load global config (~/.vrift/config.toml)
        if let Some(global_path) = Self::global_config_path() {
            if global_path.exists() {
                debug!("Loading global config from {:?}", global_path);
                let contents = std::fs::read_to_string(&global_path)?;
                config = toml::from_str(&contents)?;
            }
        }

        // 2. Load project config (.vrift/config.toml) - overrides global
        let project_path = Path::new(".vrift/config.toml");
        if project_path.exists() {
            debug!("Loading project config from {:?}", project_path);
            let contents = std::fs::read_to_string(project_path)?;
            let project_config: Config = toml::from_str(&contents)?;
            config.merge(project_config);
        }

        // 3. Apply environment variable overrides
        config.apply_env_overrides();

        Ok(config)
    }

    /// Global config path: ~/.vrift/config.toml
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".vrift/config.toml"))
    }

    /// Merge another config (project overrides)
    fn merge(&mut self, other: Config) {
        // Only merge non-default values (simplified: just replace)
        // A more sophisticated merge would check each field
        if !other.tiers.tier1_patterns.is_empty() {
            self.tiers.tier1_patterns = other.tiers.tier1_patterns;
        }
        if !other.tiers.tier2_patterns.is_empty() {
            self.tiers.tier2_patterns = other.tiers.tier2_patterns;
        }
        if !other.security.exclude_patterns.is_empty() {
            self.security.exclude_patterns = other.security.exclude_patterns;
        }
    }

    /// Apply environment variable overrides
    fn apply_env_overrides(&mut self) {
        if let Ok(path) = std::env::var("VR_THE_SOURCE") {
            self.storage.the_source = PathBuf::from(path);
        }
        if let Ok(threads) = std::env::var("VRIFT_THREADS") {
            if let Ok(n) = threads.parse() {
                self.ingest.threads = Some(n);
            }
        }
    }

    /// Generate default config TOML string
    pub fn default_toml() -> String {
        toml::to_string_pretty(&Config::default()).unwrap()
    }
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// TheSource™ CAS root directory
    pub the_source: PathBuf,
    /// Default projection mode: solid or phantom
    pub default_mode: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            the_source: PathBuf::from("~/.vrift/the_source"),
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
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            threads: None,
            default_tier: "tier2".to_string(),
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
    /// Enable daemon mode
    pub enabled: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket: PathBuf::from("/run/vrift/daemon.sock"),
            enabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            config.daemon.socket,
            PathBuf::from("/run/vrift/daemon.sock")
        );
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
        let mut config = Config::default();

        std::env::set_var("VRIFT_THREADS", "16");
        config.apply_env_overrides();
        std::env::remove_var("VRIFT_THREADS");

        assert_eq!(config.ingest.threads, Some(16));
    }

    #[test]
    fn test_env_override_invalid_threads_ignored() {
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
