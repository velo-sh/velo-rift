//! Tier classification logic for the Tiered Asset Model (RFC-0039)
//!
//! Assets are classified by write frequency for optimized protection strategies:
//! - Tier-1 (Immutable): Never written, maximum protection via symlink + chattr
//! - Tier-2 (Mutable): Rarely written, protected via hardlink + Break-Before-Write

use crate::AssetTier;

/// Default path patterns for Tier-1 (Immutable) classification
pub const DEFAULT_TIER1_PATTERNS: &[&str] = &[
    // Node.js registry deps
    "node_modules/",
    // Rust registry and toolchains
    ".cargo/registry/",
    ".rustup/",
    "/toolchains/",
    // Python environments (read-only packages)
    ".venv/lib/",
    "site-packages/",
    // System paths
    "/usr/lib/",
    "/usr/share/",
];

/// Default path patterns for Tier-2 (Mutable) classification  
pub const DEFAULT_TIER2_PATTERNS: &[&str] = &[
    // Rust build outputs
    "target/",
    "target/debug/",
    "target/release/",
    // Node.js build outputs
    "dist/",
    "build/",
    ".next/",
    // Python build outputs
    "__pycache__/",
    ".pytest_cache/",
    "*.egg-info/",
    // General cache/output
    ".cache/",
    "out/",
];

/// Configurable tier classifier
/// 
/// Can be created with custom patterns from config file,
/// or use default patterns via TierClassifier::default().
#[derive(Debug, Clone)]
pub struct TierClassifier {
    tier1_patterns: Vec<String>,
    tier2_patterns: Vec<String>,
}

impl Default for TierClassifier {
    fn default() -> Self {
        Self {
            tier1_patterns: DEFAULT_TIER1_PATTERNS.iter().map(|s| s.to_string()).collect(),
            tier2_patterns: DEFAULT_TIER2_PATTERNS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl TierClassifier {
    /// Create a new classifier with custom patterns
    pub fn new(tier1_patterns: Vec<String>, tier2_patterns: Vec<String>) -> Self {
        Self { tier1_patterns, tier2_patterns }
    }
    
    /// Classify a path into its appropriate asset tier
    pub fn classify(&self, path: &str) -> AssetTier {
        // Normalize path for matching
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{}", path)
        };

        // Check Tier-1 patterns first
        for pattern in &self.tier1_patterns {
            if normalized.contains(pattern) {
                return AssetTier::Tier1Immutable;
            }
        }

        // Tier-2 patterns are informational - default is Tier2
        for pattern in &self.tier2_patterns {
            if normalized.contains(pattern) {
                return AssetTier::Tier2Mutable;
            }
        }

        // Default: Tier-2 (mutable, safer for unclassified files)
        AssetTier::Tier2Mutable
    }
    
    /// Check if a path is a candidate for Tier-1 immutable storage
    pub fn is_immutable_candidate(&self, path: &str) -> bool {
        matches!(self.classify(path), AssetTier::Tier1Immutable)
    }
}

/// Classify a path into its appropriate asset tier (using default patterns).
///
/// Classification priority:
/// 1. Explicit Tier-1 patterns (immutable)
/// 2. Explicit Tier-2 patterns (mutable build outputs)
/// 3. Default: Tier-2 (safer default for unclassified)
///
/// # Examples
///
/// ```
/// use vrift_manifest::{classify_tier, AssetTier};
///
/// assert_eq!(classify_tier("node_modules/@types/node/index.d.ts"), AssetTier::Tier1Immutable);
/// assert_eq!(classify_tier("target/release/app"), AssetTier::Tier2Mutable);
/// assert_eq!(classify_tier("src/main.rs"), AssetTier::Tier2Mutable);
/// ```
pub fn classify_tier(path: &str) -> AssetTier {
    TierClassifier::default().classify(path)
}

/// Check if a path is a candidate for Tier-1 immutable storage
pub fn is_immutable_candidate(path: &str) -> bool {
    TierClassifier::default().is_immutable_candidate(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_modules_tier1() {
        assert_eq!(
            classify_tier("node_modules/@types/node/index.d.ts"),
            AssetTier::Tier1Immutable
        );
        assert_eq!(
            classify_tier("/project/node_modules/lodash/lodash.js"),
            AssetTier::Tier1Immutable
        );
    }

    #[test]
    fn test_cargo_registry_tier1() {
        assert_eq!(
            classify_tier(".cargo/registry/cache/some-crate.crate"),
            AssetTier::Tier1Immutable
        );
    }

    #[test]
    fn test_rustup_tier1() {
        assert_eq!(
            classify_tier(".rustup/toolchains/stable-x86_64-apple-darwin/lib/libstd.rlib"),
            AssetTier::Tier1Immutable
        );
    }

    #[test]
    fn test_target_tier2() {
        assert_eq!(
            classify_tier("target/release/my-app"),
            AssetTier::Tier2Mutable
        );
        assert_eq!(
            classify_tier("target/debug/deps/libfoo.rlib"),
            AssetTier::Tier2Mutable
        );
    }

    #[test]
    fn test_dist_tier2() {
        assert_eq!(classify_tier("dist/bundle.js"), AssetTier::Tier2Mutable);
    }

    #[test]
    fn test_source_files_tier2() {
        // Source files should default to Tier-2
        assert_eq!(classify_tier("src/main.rs"), AssetTier::Tier2Mutable);
        assert_eq!(classify_tier("lib/utils.js"), AssetTier::Tier2Mutable);
    }

    #[test]
    fn test_is_immutable_candidate() {
        assert!(is_immutable_candidate("node_modules/foo/index.js"));
        assert!(!is_immutable_candidate("target/release/app"));
        assert!(!is_immutable_candidate("src/main.rs"));
    }
}
