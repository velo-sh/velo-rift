//! Shared ignore pattern configuration for Live Ingest
//!
//! Loads ignore patterns from vrift-config [ingest] section.

use std::path::Path;

/// Ignore pattern matcher
#[derive(Debug, Clone)]
pub struct IgnoreMatcher {
    patterns: Vec<String>,
}

impl Default for IgnoreMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl IgnoreMatcher {
    /// Create a new matcher with patterns from config
    pub fn new() -> Self {
        // Load entirely from config - no hardcoded fallback
        let patterns = vrift_config::config().ingest.ignore_patterns.clone();
        Self { patterns }
    }

    /// Create a matcher with custom patterns
    pub fn with_patterns(patterns: &[String]) -> Self {
        Self {
            patterns: patterns.to_vec(),
        }
    }

    /// Check if a path should be ignored
    pub fn should_ignore(&self, path: &Path) -> bool {
        for pattern in &self.patterns {
            // Glob pattern (e.g., *.pyc)
            if let Some(suffix) = pattern.strip_prefix('*') {
                if path
                    .extension()
                    .is_some_and(|ext| format!(".{}", ext.to_string_lossy()) == suffix)
                {
                    return true;
                }
            }
            // Directory/file name match
            else if path
                .components()
                .any(|c| c.as_os_str().to_string_lossy() == *pattern)
            {
                return true;
            }
        }
        false
    }

    /// Get the patterns
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_ignore_git() {
        // Use explicit patterns - .git is NOT in code defaults
        let matcher = IgnoreMatcher::with_patterns(&[".git".to_string()]);
        assert!(matcher.should_ignore(&PathBuf::from("/project/.git/config")));
    }

    #[test]
    fn test_custom_patterns() {
        let matcher = IgnoreMatcher::with_patterns(&["custom".to_string()]);
        assert!(matcher.should_ignore(&PathBuf::from("custom/file.txt")));
        assert!(!matcher.should_ignore(&PathBuf::from("other/file.txt")));
    }
}
