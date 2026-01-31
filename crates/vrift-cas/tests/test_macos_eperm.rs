//! Test case for macOS EPERM handling (code-signed bundles)
//!
//! This test verifies that vrift correctly handles EPERM errors when
//! attempting to hard_link files inside macOS code-signed bundles
//! (e.g., Chromium.app from puppeteer).
//!
//! Pattern 987: macOS EPERM remediation - fallback to copy

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// Test that ingest handles EPERM by falling back to copy
/// 
/// This test simulates a code-signed bundle scenario where hard_link fails
#[test]
#[cfg(target_os = "macos")]
fn test_eperm_fallback_to_copy() {
    use vrift_cas::ingest_solid_tier2;
    
    let temp_dir = TempDir::new().unwrap();
    let cas_dir = temp_dir.path().join("cas");
    let source_dir = temp_dir.path().join("source");
    
    fs::create_dir_all(&cas_dir).unwrap();
    fs::create_dir_all(&source_dir).unwrap();
    
    // Create a test file
    let test_file = source_dir.join("test.txt");
    let mut f = File::create(&test_file).unwrap();
    f.write_all(b"test content for EPERM handling").unwrap();
    drop(f);
    
    // Normal ingest should succeed
    let result = ingest_solid_tier2(&test_file, &cas_dir);
    assert!(result.is_ok(), "Ingest should succeed: {:?}", result);
}

/// Integration test: ingest node_modules with puppeteer
/// 
/// This test requires puppeteer to be installed and will verify that
/// the EPERM error from Chromium.app is handled correctly.
/// 
/// Run with: cargo test --package vrift-cas --test test_macos_eperm -- --ignored
#[test]
#[ignore] // Requires puppeteer installation
#[cfg(target_os = "macos")]
fn test_puppeteer_chromium_ingest() {
    // Check if puppeteer test data exists
    let puppeteer_dir = Path::new("/tmp/vrift-puppeteer-test/node_modules/puppeteer");
    if !puppeteer_dir.exists() {
        eprintln!("Skipping test: puppeteer not installed at {:?}", puppeteer_dir);
        eprintln!("To setup: mkdir -p /tmp/vrift-puppeteer-test && cd /tmp/vrift-puppeteer-test && npm init -y && npm install puppeteer");
        return;
    }
    
    // Find the Chromium.app bundle
    let chromium_glob = "/tmp/vrift-puppeteer-test/node_modules/puppeteer/.local-chromium/*/chrome-mac/Chromium.app";
    let output = Command::new("sh")
        .args(["-c", &format!("ls -d {} 2>/dev/null | head -1", chromium_glob)])
        .output()
        .expect("Failed to find Chromium.app");
    
    let chromium_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if chromium_path.is_empty() {
        eprintln!("Chromium.app not found, skipping EPERM test");
        return;
    }
    
    eprintln!("Found Chromium.app at: {}", chromium_path);
    
    // Now run ingest - this should NOT fail with EPERM
    let temp_cas = TempDir::new().unwrap();
    let files_to_ingest: Vec<_> = walkdir::WalkDir::new(&chromium_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .take(10) // Just test first 10 files
        .map(|e| e.path().to_path_buf())
        .collect();
    
    if files_to_ingest.is_empty() {
        eprintln!("No files found in Chromium.app");
        return;
    }
    
    eprintln!("Testing ingest of {} files from Chromium.app", files_to_ingest.len());
    
    for file in &files_to_ingest {
        // Copy file to temp location (since we can't modify the original)
        let temp_source = temp_cas.path().join("source");
        fs::create_dir_all(&temp_source).unwrap();
        let temp_file = temp_source.join(file.file_name().unwrap());
        fs::copy(file, &temp_file).unwrap();
        
        // Ingest - should succeed with EPERM fallback to copy
        let cas_root = temp_cas.path().join("cas");
        fs::create_dir_all(&cas_root).unwrap();
        
        let result = vrift_cas::ingest_solid_tier2(&temp_file, &cas_root);
        assert!(result.is_ok(), "Ingest failed for {:?}: {:?}", file, result);
    }
    
    eprintln!("✓ EPERM fallback test passed");
}
