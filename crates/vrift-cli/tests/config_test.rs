//! Integration tests for vrift config commands

use std::process::Command;

/// Helper to run vrift command
fn vrift(args: &[&str]) -> std::process::Output {
    Command::new("cargo")
        .args([
            "run",
            "--package",
            "vrift-cli",
            "--bin",
            "vrift",
            "--quiet",
            "--",
        ])
        .args(args)
        .output()
        .expect("Failed to execute vrift")
}

#[test]
fn test_config_help() {
    let output = vrift(&["config", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success() || stdout.contains("Configuration"));
    assert!(stdout.contains("init") || stdout.contains("show") || stdout.contains("path"));
}

#[test]
fn test_config_path() {
    let output = vrift(&["config", "path"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Global:"));
    assert!(stdout.contains("Project:"));
}

#[test]
fn test_config_show_outputs_toml() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    // Should output valid TOML sections
    assert!(stdout.contains("[storage]"));
    assert!(stdout.contains("[tiers]"));
    assert!(stdout.contains("[security]"));
}

#[test]
fn test_config_show_contains_all_sections() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("[storage]"));
    assert!(stdout.contains("[ingest]"));
    assert!(stdout.contains("[tiers]"));
    assert!(stdout.contains("[security]"));
    assert!(stdout.contains("[daemon]"));
}

#[test]
fn test_config_show_contains_default_patterns() {
    let output = vrift(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("node_modules/"));
    assert!(stdout.contains(".cargo/registry/"));
    assert!(stdout.contains("target/"));
}

// Note: The following tests require running vrift from a temp directory
// which doesn't work well with `cargo run`. These are marked as ignored
// and can be run manually after building the binary.

#[test]
#[ignore = "Requires pre-built binary and special test setup"]
fn test_config_init_creates_local_config() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new("cargo")
        .args(["run", "--package", "vrift-cli", "--bin", "vrift", "--"])
        .args(["config", "init"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute vrift");

    assert!(output.status.success());

    let config_path = temp.path().join(".vrift/config.toml");
    assert!(config_path.exists());
}

#[test]
#[ignore = "Requires pre-built binary and special test setup"]
fn test_config_init_fails_if_exists() {
    let temp = tempfile::tempdir().unwrap();

    let config_dir = temp.path().join(".vrift");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), "# existing").unwrap();

    let output = Command::new("cargo")
        .args(["run", "--package", "vrift-cli", "--bin", "vrift", "--"])
        .args(["config", "init"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute vrift");

    assert!(!output.status.success());
}

#[test]
#[ignore = "Requires pre-built binary and special test setup"]
fn test_config_init_force_overwrites() {
    let temp = tempfile::tempdir().unwrap();

    let config_dir = temp.path().join(".vrift");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), "# old content").unwrap();

    let output = Command::new("cargo")
        .args(["run", "--package", "vrift-cli", "--bin", "vrift", "--"])
        .args(["config", "init", "--force"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute vrift");

    assert!(output.status.success());
}
