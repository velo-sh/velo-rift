//! # Preflight Checks for Inception Mode
//!
//! Fail-fast, fail-early validation before shim is loaded.
//! All dependencies must be validated BEFORE inception activates.

use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use console::style;

/// VDIR_MAGIC must match vrift-vdird/src/vdir.rs
const VDIR_MAGIC: u32 = 0x56524654; // "VRFT"
/// VDIR_VERSION must match vrift-vdird/src/vdir.rs
const VDIR_VERSION: u32 = 4; // Must match vrift-ipc/src/vdir_types.rs

/// Result of preflight checks
#[derive(Debug)]
pub struct PreflightResult {
    pub can_activate: bool,
    pub vdir_path: PathBuf,
    pub socket_path: PathBuf,
    pub project_root: PathBuf,
    pub errors: Vec<String>,
}

impl Default for PreflightResult {
    fn default() -> Self {
        Self {
            can_activate: false,
            vdir_path: PathBuf::new(),
            socket_path: PathBuf::new(),
            project_root: PathBuf::new(),
            errors: Vec::new(),
        }
    }
}

/// Run all preflight checks for inception mode
///
/// Returns PreflightResult with can_activate=true only if ALL checks pass.
pub fn run_preflight(project_dir: &Path) -> PreflightResult {
    let mut result = PreflightResult::default();

    // Resolve project root
    let project_root = match project_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            result
                .errors
                .push(format!("Cannot resolve project path: {}", e));
            return result;
        }
    };
    result.project_root = project_root.clone();

    let vrift_dir = project_root.join(".vrift");

    // Check 1: .vrift directory exists
    if !vrift_dir.exists() {
        result.errors.push(format!(
            "Project not initialized. Run: {}",
            style("vrift init").cyan()
        ));
        return result;
    }

    // Check 2: VDir file exists (global path: ~/.vrift/vdir/<project_hash>.vdir)
    let project_id = vrift_config::path::compute_project_id(&project_root);
    let vdir_path = match vrift_config::path::get_vdir_mmap_path(&project_id) {
        Some(p) => p,
        None => {
            result
                .errors
                .push("Cannot determine VDir path from config".to_string());
            return result;
        }
    };
    if !vdir_path.exists() {
        result.errors.push(format!(
            "VDir not found at {}. Run: {} first",
            vdir_path.display(),
            style("vrift ingest <project>").cyan()
        ));
        return result;
    }
    result.vdir_path = vdir_path.clone();

    // Check 3: VDir version matches
    if let Err(e) = validate_vdir_version(&vdir_path) {
        result.errors.push(e);
        return result;
    }

    // Check 4: Socket exists and daemon responds
    let cfg = vrift_config::config();
    let socket_path = cfg.socket_path().to_path_buf();
    if !socket_path.exists() {
        result.errors.push(format!(
            "Daemon socket not found. Run: {}",
            style("vrift daemon start").cyan()
        ));
        return result;
    }
    result.socket_path = socket_path.clone();

    // Check 5: Socket is connectable (daemon is alive)
    if let Err(e) = test_socket_connection(&socket_path) {
        result.errors.push(e);
        return result;
    }

    // Check 6: CAS root is configured and writable
    if let Err(e) = check_cas_writable() {
        result.errors.push(e);
        return result;
    }

    // Check 7: Staging directory is writable
    let staging_path = vrift_dir.join("staging");
    if let Err(e) = check_dir_writable(&staging_path, "Staging directory") {
        result.errors.push(e);
        return result;
    }

    // All checks passed!
    result.can_activate = true;
    result
}

/// Validate VDir file magic and version
fn validate_vdir_version(vdir_path: &Path) -> Result<(), String> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(vdir_path).map_err(|e| format!("Cannot open VDir: {}", e))?;

    let mut header = [0u8; 8]; // magic (4) + version (4)
    file.read_exact(&mut header)
        .map_err(|e| format!("Cannot read VDir header: {}", e))?;

    let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);

    if magic != VDIR_MAGIC {
        return Err(format!(
            "VDir corrupted (invalid magic). Run: {}",
            style("vrift init --force").cyan()
        ));
    }

    if version != VDIR_VERSION {
        return Err(format!(
            "VDir version mismatch (have: {}, need: {}). Run: {}",
            version,
            VDIR_VERSION,
            style("vrift init --force").cyan()
        ));
    }

    Ok(())
}

/// Test if daemon socket is connectable
fn test_socket_connection(socket_path: &Path) -> Result<(), String> {
    let stream = UnixStream::connect(socket_path).map_err(|e| {
        format!(
            "Daemon not responding: {}. Run: {}",
            e,
            style("vrift daemon start").cyan()
        )
    })?;

    // Set timeout for the connection test
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok();

    // Clean shutdown
    let _ = stream.shutdown(Shutdown::Both);

    Ok(())
}

/// Check if CAS root is configured and writable
fn check_cas_writable() -> Result<(), String> {
    let cfg = vrift_config::config();
    let cas_root_str = cfg.cas_root().display().to_string();

    let cas_path = Path::new(&cas_root_str);

    // CAS doesn't need to exist yet (will be created), but parent must be writable
    if cas_path.exists() {
        check_dir_writable(cas_path, "CAS root")?;
    } else if let Some(parent) = cas_path.parent() {
        if parent.exists() {
            check_dir_writable(parent, "CAS parent directory")?;
        }
        // If parent doesn't exist, it will be created on first write
    }

    Ok(())
}

/// Check if directory is writable
fn check_dir_writable(path: &Path, name: &str) -> Result<(), String> {
    if !path.exists() {
        // Directory doesn't exist - that's OK, it will be created
        return Ok(());
    }

    // Try to create a temp file to verify write access
    let test_file = path.join(".vrift_write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_file);
            Ok(())
        }
        Err(e) => Err(format!("{} not writable: {}", name, e)),
    }
}

/// Print preflight errors to stderr with nice formatting
pub fn print_preflight_errors(result: &PreflightResult) {
    eprintln!();
    eprintln!(
        "{} {}",
        style("❌").red(),
        style("Preflight checks failed").red().bold()
    );
    eprintln!();

    for (i, error) in result.errors.iter().enumerate() {
        eprintln!("   {}. {}", i + 1, error);
    }

    eprintln!();
    eprintln!(
        "{}",
        style("Inception mode requires all checks to pass.").dim()
    );
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_preflight_missing_vrift_dir() {
        let dir = tempdir().unwrap();
        let result = run_preflight(dir.path());
        assert!(!result.can_activate);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].contains("not initialized"));
    }

    #[test]
    fn test_preflight_missing_vdir() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vrift")).unwrap();
        let result = run_preflight(dir.path());
        assert!(!result.can_activate);
        assert!(result.errors[0].contains("VDir not found"));
    }

    #[test]
    fn test_check_dir_writable_nonexistent() {
        let result = check_dir_writable(Path::new("/nonexistent/path"), "Test");
        assert!(result.is_ok()); // Non-existent is OK (will be created)
    }
}
