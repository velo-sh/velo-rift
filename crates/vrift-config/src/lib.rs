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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub storage: StorageConfig,
    pub ingest: IngestConfig,
    pub tiers: TierConfig,
    pub security: SecurityConfig,
    pub daemon: DaemonConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            storage: StorageConfig::default(),
            ingest: IngestConfig::default(),
            tiers: TierConfig::default(),
            security: SecurityConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
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

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(!config.tiers.tier1_patterns.is_empty());
        assert!(!config.security.exclude_patterns.is_empty());
    }

    #[test]
    fn test_default_toml_generation() {
        let toml_str = Config::default_toml();
        assert!(toml_str.contains("[storage]"));
        assert!(toml_str.contains("[tiers]"));
        assert!(toml_str.contains("node_modules/"));
    }

    #[test]
    fn test_toml_roundtrip() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            config.tiers.tier1_patterns.len(),
            parsed.tiers.tier1_patterns.len()
        );
    }
}
