//! # Security Filter (RFC-0042)
//!
//! Auto-excludes sensitive files during ingest to prevent accidental
//! exposure of secrets, credentials, and private keys.

use std::path::Path;

/// Default patterns for sensitive files that should be excluded from ingest.
/// See RFC-0042 for full rationale and pattern list.
const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
    // Environment & Secrets
    ".env",
    ".env.local",
    ".env.development",
    ".env.production",
    ".env.staging",
    ".env.test",
    "secrets.yaml",
    "secrets.yml",
    "secrets.json",
    "secrets.toml",
    // Credentials
    ".npmrc",
    ".netrc",
    ".git-credentials",
    ".pypirc",
    // Private Keys
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    // VRift Internal
    ".vrift",
    ".git",
];

/// File extensions that indicate private keys or certificates
const SENSITIVE_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "jks", "ppk"];

/// Directory names that contain credentials
const SENSITIVE_DIRS: &[&str] = &[".aws", ".docker", ".secrets", ".ssh"];

/// Security filter for ingest operations
pub struct SecurityFilter {
    enabled: bool,
    excluded_files: Vec<String>,
}

impl SecurityFilter {
    /// Create a new security filter (enabled by default)
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            excluded_files: Vec::new(),
        }
    }

    /// Check if a path should be excluded from ingest
    /// Returns (should_exclude, reason) tuple
    pub fn should_exclude(&self, path: &Path) -> Option<&'static str> {
        if !self.enabled {
            return None;
        }

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Check exact filename matches
        for pattern in DEFAULT_EXCLUDE_PATTERNS {
            if file_name == *pattern {
                return Some(match *pattern {
                    ".env" | ".env.local" | ".env.development" | ".env.production"
                    | ".env.staging" | ".env.test" => "environment secrets",
                    "secrets.yaml" | "secrets.yml" | "secrets.json" | "secrets.toml" => {
                        "secrets file"
                    }
                    ".npmrc" | ".netrc" | ".git-credentials" | ".pypirc" => "credentials",
                    "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa" => "SSH private key",
                    ".vrift" => "VRift internal",
                    ".git" => "Git internal",
                    _ => "sensitive file",
                });
            }
        }

        // Check .env.* pattern (covers .env.anything)
        if file_name.starts_with(".env.") {
            return Some("environment secrets");
        }

        // Check *.secret pattern
        if file_name.ends_with(".secret") {
            return Some("secret file");
        }

        // Check sensitive extensions
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_lowercase();
            for sensitive_ext in SENSITIVE_EXTENSIONS {
                if ext_lower == *sensitive_ext {
                    return Some(match ext_lower.as_str() {
                        "pem" | "key" => "private key",
                        "p12" | "pfx" | "jks" => "certificate bundle",
                        "ppk" => "PuTTY private key",
                        _ => "sensitive file",
                    });
                }
            }
        }

        // Check if path contains sensitive directories
        for component in path.components() {
            if let Some(name) = component.as_os_str().to_str() {
                for sensitive_dir in SENSITIVE_DIRS {
                    if name == *sensitive_dir {
                        return Some(match name {
                            ".aws" => "AWS credentials",
                            ".docker" => "Docker credentials",
                            ".secrets" => "secrets directory",
                            ".ssh" => "SSH credentials",
                            _ => "sensitive directory",
                        });
                    }
                }
            }
        }

        None
    }

    /// Record an excluded file for reporting
    pub fn record_exclusion(&mut self, path: &Path) {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            self.excluded_files.push(name.to_string());
        }
    }

    /// Get list of excluded file names
    #[allow(dead_code)]
    pub fn excluded_files(&self) -> &[String] {
        &self.excluded_files
    }

    /// Get count of excluded files
    pub fn excluded_count(&self) -> usize {
        self.excluded_files.len()
    }

    /// Whether the filter is enabled
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_files() {
        let filter = SecurityFilter::new(true);

        assert!(filter.should_exclude(Path::new(".env")).is_some());
        assert!(filter.should_exclude(Path::new(".env.local")).is_some());
        assert!(filter
            .should_exclude(Path::new(".env.production"))
            .is_some());
        assert!(filter.should_exclude(Path::new(".env.custom")).is_some());
    }

    #[test]
    fn test_private_keys() {
        let filter = SecurityFilter::new(true);

        assert!(filter.should_exclude(Path::new("id_rsa")).is_some());
        assert!(filter.should_exclude(Path::new("server.key")).is_some());
        assert!(filter.should_exclude(Path::new("cert.pem")).is_some());
        assert!(filter.should_exclude(Path::new("keystore.p12")).is_some());
    }

    #[test]
    fn test_sensitive_dirs() {
        let filter = SecurityFilter::new(true);

        assert!(filter
            .should_exclude(Path::new(".aws/credentials"))
            .is_some());
        assert!(filter
            .should_exclude(Path::new(".docker/config.json"))
            .is_some());
        assert!(filter.should_exclude(Path::new(".ssh/id_rsa")).is_some());
    }

    #[test]
    fn test_safe_files() {
        let filter = SecurityFilter::new(true);

        assert!(filter.should_exclude(Path::new("package.json")).is_none());
        assert!(filter.should_exclude(Path::new("index.js")).is_none());
        assert!(filter.should_exclude(Path::new("README.md")).is_none());
    }

    #[test]
    fn test_disabled_filter() {
        let filter = SecurityFilter::new(false);

        // When disabled, nothing should be excluded
        assert!(filter.should_exclude(Path::new(".env")).is_none());
        assert!(filter.should_exclude(Path::new("id_rsa")).is_none());
    }
}
