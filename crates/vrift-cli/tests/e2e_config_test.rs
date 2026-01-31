//! E2E tests for vrift config system
//!
//! These tests verify the complete workflow from config setup to actual usage.

use std::process::Command;

/// Helper to run vrift from project root
fn vrift(args: &[&str]) -> std::process::Output {
    Command::new("cargo")
        .args(["run", "--package", "vrift-cli", "--quiet", "--"])
        .args(args)
        .output()
        .expect("Failed to execute vrift")
}

// ========== E2E: Config Show Workflow ==========

#[test]
fn e2e_config_show_returns_valid_toml() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());

    // Verify it's valid TOML by parsing
    let parsed: Result<vrift_config::Config, _> = toml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "config show should return valid TOML: {:?}",
        parsed.err()
    );

    let config = parsed.unwrap();
    assert!(!config.tiers.tier1_patterns.is_empty());
}

#[test]
fn e2e_config_show_contains_expected_sections() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());

    // All expected sections present
    assert!(stdout.contains("[storage]"), "Missing [storage] section");
    assert!(stdout.contains("[ingest]"), "Missing [ingest] section");
    assert!(stdout.contains("[tiers]"), "Missing [tiers] section");
    assert!(stdout.contains("[security]"), "Missing [security] section");
    assert!(stdout.contains("[daemon]"), "Missing [daemon] section");

    // Key config values present
    assert!(
        stdout.contains("node_modules/"),
        "Missing node_modules pattern"
    );
    assert!(stdout.contains("target/"), "Missing target pattern");
    assert!(stdout.contains(".env"), "Missing .env security pattern");
}

// ========== E2E: Config Path Workflow ==========

#[test]
fn e2e_config_path_shows_locations() {
    let output = vrift(&["config", "path"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Global:"), "Missing Global config path");
    assert!(stdout.contains("Project:"), "Missing Project config path");
    assert!(
        stdout.contains(".vrift/config.toml"),
        "Missing config.toml in path"
    );
}

// ========== E2E: Tier Classification Verification ==========

#[test]
fn e2e_config_tier_patterns_applied() {
    use vrift_manifest::tier::TierClassifier;
    use vrift_manifest::AssetTier;

    // Get current config
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    let config: vrift_config::Config = toml::from_str(&stdout).unwrap();

    // Create classifier from config patterns
    let classifier = TierClassifier::new(
        config.tiers.tier1_patterns.clone(),
        config.tiers.tier2_patterns.clone(),
    );

    // Verify expected classifications based on default config
    // node_modules should be tier1
    assert_eq!(
        classifier.classify("node_modules/lodash/index.js"),
        AssetTier::Tier1Immutable,
        "node_modules should be Tier1"
    );

    // target should be tier2
    assert_eq!(
        classifier.classify("target/release/app"),
        AssetTier::Tier2Mutable,
        "target should be Tier2"
    );

    // .cargo/registry should be tier1
    assert_eq!(
        classifier.classify(".cargo/registry/cache/crate.tar.gz"),
        AssetTier::Tier1Immutable,
        ".cargo/registry should be Tier1"
    );
}

// ========== E2E: Security Filter Verification ==========

#[test]
fn e2e_config_security_patterns_present() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    let config: vrift_config::Config = toml::from_str(&stdout).unwrap();

    // Security should be enabled by default
    assert!(
        config.security.enabled,
        "Security filter should be enabled by default"
    );

    // Key security patterns should be present
    let patterns = &config.security.exclude_patterns;
    assert!(
        patterns.iter().any(|p| p.contains("env")),
        "Should exclude .env files"
    );
    assert!(
        patterns.iter().any(|p| p.contains("key")),
        "Should exclude key files"
    );
    assert!(
        patterns.iter().any(|p| p.contains("pem")),
        "Should exclude pem files"
    );
}

// ========== E2E: Default Values Verification ==========

#[test]
fn e2e_config_defaults_are_sensible() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    let config: vrift_config::Config = toml::from_str(&stdout).unwrap();

    // Storage defaults
    assert_eq!(
        config.storage.default_mode, "solid",
        "Default mode should be solid"
    );

    // Ingest defaults
    assert_eq!(
        config.ingest.default_tier, "tier2",
        "Default tier should be tier2"
    );

    // Daemon should be disabled by default
    assert!(
        !config.daemon.enabled,
        "Daemon should be disabled by default"
    );

    // Reasonable number of patterns
    assert!(
        config.tiers.tier1_patterns.len() >= 5,
        "Should have at least 5 tier1 patterns"
    );
    assert!(
        config.tiers.tier2_patterns.len() >= 5,
        "Should have at least 5 tier2 patterns"
    );
    assert!(
        config.security.exclude_patterns.len() >= 5,
        "Should have at least 5 security patterns"
    );
}

// ========== E2E: Multi-Language Support ==========

#[test]
fn e2e_config_supports_multiple_languages() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    let config: vrift_config::Config = toml::from_str(&stdout).unwrap();

    let tier1 = &config.tiers.tier1_patterns;

    // Node.js
    assert!(
        tier1.iter().any(|p| p.contains("node_modules")),
        "Should support Node.js"
    );

    // Rust
    assert!(
        tier1.iter().any(|p| p.contains(".cargo")),
        "Should support Rust cargo"
    );
    assert!(
        tier1.iter().any(|p| p.contains(".rustup")),
        "Should support Rust rustup"
    );

    // Python
    assert!(
        tier1.iter().any(|p| p.contains("site-packages")),
        "Should support Python site-packages"
    );
    assert!(
        tier1.iter().any(|p| p.contains(".venv")),
        "Should support Python venv"
    );
}
