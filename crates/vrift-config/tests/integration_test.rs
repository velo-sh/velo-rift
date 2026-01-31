//! Integration tests for vrift-config
//!
//! These tests verify the full config loading pipeline with real file system operations.

use std::path::PathBuf;
use tempfile::tempdir;

/// Test config loading from real global config file
#[test]
fn test_load_global_config_from_file() {
    let temp = tempdir().unwrap();
    let vrift_dir = temp.path().join(".vrift");
    std::fs::create_dir_all(&vrift_dir).unwrap();

    let config_content = r#"
[storage]
the_source = "/custom/the_source"
default_mode = "phantom"

[ingest]
threads = 4
default_tier = "tier1"

[tiers]
tier1_patterns = ["vendor/", "libs/"]
tier2_patterns = ["build/", "out/"]
"#;
    std::fs::write(vrift_dir.join("config.toml"), config_content).unwrap();

    // Read and parse
    let contents = std::fs::read_to_string(vrift_dir.join("config.toml")).unwrap();
    let config: vrift_config::Config = toml::from_str(&contents).unwrap();

    assert_eq!(
        config.storage.the_source,
        PathBuf::from("/custom/the_source")
    );
    assert_eq!(config.storage.default_mode, "phantom");
    assert_eq!(config.ingest.threads, Some(4));
    assert_eq!(config.ingest.default_tier, "tier1");
    assert_eq!(config.tiers.tier1_patterns, vec!["vendor/", "libs/"]);
    assert_eq!(config.tiers.tier2_patterns, vec!["build/", "out/"]);
}

/// Test config hierarchy: project config overrides global
#[test]
fn test_config_hierarchy_project_overrides_global() {
    let temp = tempdir().unwrap();

    // Create "global" config
    let global_dir = temp.path().join("global/.vrift");
    std::fs::create_dir_all(&global_dir).unwrap();
    let global_config = r#"
[tiers]
tier1_patterns = ["node_modules/", ".cargo/"]
tier2_patterns = ["target/"]

[security]
exclude_patterns = [".env", "*.key"]
"#;
    std::fs::write(global_dir.join("config.toml"), global_config).unwrap();

    // Create "project" config with overrides
    let project_dir = temp.path().join("project/.vrift");
    std::fs::create_dir_all(&project_dir).unwrap();
    let project_config = r#"
[tiers]
tier1_patterns = ["vendor/", "third_party/"]
"#;
    std::fs::write(project_dir.join("config.toml"), project_config).unwrap();

    // Load global first
    let global_contents = std::fs::read_to_string(global_dir.join("config.toml")).unwrap();
    let mut config: vrift_config::Config = toml::from_str(&global_contents).unwrap();

    // Load and merge project config
    let project_contents = std::fs::read_to_string(project_dir.join("config.toml")).unwrap();
    let project: vrift_config::Config = toml::from_str(&project_contents).unwrap();

    // Simulate merge (tier1 patterns should be replaced)
    if !project.tiers.tier1_patterns.is_empty() {
        config.tiers.tier1_patterns = project.tiers.tier1_patterns;
    }

    // Verify: tier1 replaced, tier2 preserved from global, security preserved
    assert_eq!(config.tiers.tier1_patterns, vec!["vendor/", "third_party/"]);
    assert_eq!(config.tiers.tier2_patterns, vec!["target/"]);
    assert!(config
        .security
        .exclude_patterns
        .iter()
        .any(|p| p.contains(".env")));
}

/// Test TierClassifier integration with config patterns
#[test]
fn test_tier_classifier_with_config_patterns() {
    use vrift_manifest::tier::TierClassifier;
    use vrift_manifest::AssetTier;

    // Load config with custom patterns
    let config_toml = r#"
[tiers]
tier1_patterns = ["frozen/", "immutable/"]
tier2_patterns = ["mutable/", "temp/"]
"#;
    let config: vrift_config::Config = toml::from_str(config_toml).unwrap();

    // Create classifier from config
    let classifier = TierClassifier::new(
        config.tiers.tier1_patterns.clone(),
        config.tiers.tier2_patterns.clone(),
    );

    // Verify classification
    assert_eq!(
        classifier.classify("frozen/package.json"),
        AssetTier::Tier1Immutable
    );
    assert_eq!(
        classifier.classify("immutable/lib.so"),
        AssetTier::Tier1Immutable
    );
    assert_eq!(
        classifier.classify("mutable/cache.db"),
        AssetTier::Tier2Mutable
    );
    assert_eq!(
        classifier.classify("temp/session.tmp"),
        AssetTier::Tier2Mutable
    );

    // Unmatched defaults to Tier2
    assert_eq!(classifier.classify("src/main.rs"), AssetTier::Tier2Mutable);
}

/// Test config with environment variable override
#[test]
fn test_config_env_override_integration() {
    let config_toml = r#"
[storage]
the_source = "~/.vrift/the_source"

[ingest]
threads = 2
"#;
    let mut config: vrift_config::Config = toml::from_str(config_toml).unwrap();

    // Set env vars
    std::env::set_var("VR_THE_SOURCE", "/override/path");
    std::env::set_var("VRIFT_THREADS", "16");

    // Apply overrides (simulating what Config::apply_env_overrides does)
    if let Ok(path) = std::env::var("VR_THE_SOURCE") {
        config.storage.the_source = PathBuf::from(path);
    }
    if let Ok(threads) = std::env::var("VRIFT_THREADS") {
        if let Ok(n) = threads.parse() {
            config.ingest.threads = Some(n);
        }
    }

    // Cleanup
    std::env::remove_var("VR_THE_SOURCE");
    std::env::remove_var("VRIFT_THREADS");

    // Verify overrides applied
    assert_eq!(config.storage.the_source, PathBuf::from("/override/path"));
    assert_eq!(config.ingest.threads, Some(16));
}

/// Test complete config serialization/deserialization cycle
#[test]
fn test_config_full_roundtrip_with_all_sections() {
    let original = vrift_config::Config::default();

    // Write to temp file
    let temp = tempdir().unwrap();
    let config_path = temp.path().join("config.toml");
    let toml_str = toml::to_string_pretty(&original).unwrap();
    std::fs::write(&config_path, &toml_str).unwrap();

    // Read back
    let contents = std::fs::read_to_string(&config_path).unwrap();
    let loaded: vrift_config::Config = toml::from_str(&contents).unwrap();

    // Verify all sections match
    assert_eq!(original.storage.default_mode, loaded.storage.default_mode);
    assert_eq!(original.ingest.default_tier, loaded.ingest.default_tier);
    assert_eq!(
        original.tiers.tier1_patterns.len(),
        loaded.tiers.tier1_patterns.len()
    );
    assert_eq!(
        original.tiers.tier2_patterns.len(),
        loaded.tiers.tier2_patterns.len()
    );
    assert_eq!(original.security.enabled, loaded.security.enabled);
    assert_eq!(
        original.security.exclude_patterns.len(),
        loaded.security.exclude_patterns.len()
    );
    assert_eq!(original.daemon.enabled, loaded.daemon.enabled);
}

/// Test partial config with defaults filling in
#[test]
fn test_partial_config_defaults_applied() {
    let partial = r#"
[storage]
default_mode = "phantom"
"#;
    let config: vrift_config::Config = toml::from_str(partial).unwrap();

    // Specified values
    assert_eq!(config.storage.default_mode, "phantom");

    // Defaults applied
    assert!(config
        .tiers
        .tier1_patterns
        .iter()
        .any(|p| p.contains("node_modules")));
    assert!(config.security.enabled);
    assert!(!config.daemon.enabled);
}
