//! Active projection mode implementation for Velo Rift™
//!
//! Implements `vrift active` command for RFC-0039 Transparent Virtual Projection.
//! Creates a persistent session that projects dependencies from CAS.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use vrift_config::path::normalize_for_ipc;

/// Session state persisted to `.vrift/session.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Session creation timestamp (Unix epoch)
    pub created_at: u64,

    /// Operational mode
    pub mode: ProjectionMode,

    /// ABI context for binary compatibility
    pub abi_context: AbiContext,

    /// Project root directory (absolute path)
    pub project_root: PathBuf,

    /// Whether the session is active
    pub active: bool,
}

/// Projection mode (Solid vs Phantom)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProjectionMode {
    /// Physical files remain in project (safe rollback)
    #[default]
    Solid,

    /// Pure virtual projection (requires restoration)
    Phantom,
}

impl std::fmt::Display for ProjectionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionMode::Solid => write!(f, "Solid"),
            ProjectionMode::Phantom => write!(f, "Phantom"),
        }
    }
}

/// ABI context for binary artifact compatibility
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AbiContext {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target_triple: String,

    /// Detected toolchain version
    pub toolchain_version: Option<String>,

    /// Python version if applicable
    pub python_version: Option<String>,

    /// Node.js version if applicable
    pub node_version: Option<String>,
}

/// Velo Rift directory structure
pub struct VriftDir {
    /// Root path (usually `.vrift/`)
    pub root: PathBuf,
}

impl VriftDir {
    /// Standard directory name
    pub const DIR_NAME: &'static str = ".vrift";

    /// Create a VriftDir for the given project root
    pub fn new(project_root: &Path) -> Self {
        Self {
            root: project_root.join(Self::DIR_NAME),
        }
    }

    /// Create the directory structure if it doesn't exist
    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("Failed to create {}", self.root.display()))?;

        // Create manifest.lmdb directory (LMDB needs a directory)
        fs::create_dir_all(self.manifest_path()).context("Failed to create manifest directory")?;

        Ok(())
    }

    /// Path to session.json
    pub fn session_path(&self) -> PathBuf {
        self.root.join("session.json")
    }

    /// Path to manifest.lmdb (directory)
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.lmdb")
    }

    /// Check if a session exists
    pub fn has_session(&self) -> bool {
        self.session_path().exists()
    }

    /// Load existing session
    pub fn load_session(&self) -> Result<Session> {
        let content = fs::read_to_string(self.session_path())
            .with_context(|| "Failed to read session.json")?;
        let session: Session =
            serde_json::from_str(&content).with_context(|| "Failed to parse session.json")?;
        Ok(session)
    }

    /// Save session
    pub fn save_session(&self, session: &Session) -> Result<()> {
        let content =
            serde_json::to_string_pretty(session).with_context(|| "Failed to serialize session")?;
        fs::write(self.session_path(), content).with_context(|| "Failed to write session.json")?;
        Ok(())
    }
}

/// Detect ABI context from the current environment
pub fn detect_abi_context() -> AbiContext {
    let target_triple = std::env::consts::ARCH.to_string() + "-" + std::env::consts::OS;

    AbiContext {
        target_triple,
        toolchain_version: detect_rust_version(),
        python_version: detect_python_version(),
        node_version: detect_node_version(),
    }
}

fn detect_rust_version() -> Option<String> {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn detect_python_version() -> Option<String> {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn detect_node_version() -> Option<String> {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

/// Activate Velo projection for a project directory
pub fn activate(project_root: &Path, mode: ProjectionMode) -> Result<Session> {
    let project_root = normalize_for_ipc(project_root)
        .with_context(|| format!("Invalid project path: {}", project_root.display()))?;

    let vrift = VriftDir::new(&project_root);

    // Create directory structure
    vrift.ensure()?;

    // Check for existing session
    if vrift.has_session() {
        let existing = vrift.load_session()?;
        if existing.active {
            info!("Velo is already active in [{}] mode", existing.mode);
            return Ok(existing);
        }
    }

    // Detect ABI context
    let abi_context = detect_abi_context();
    debug!(?abi_context, "Detected ABI context");

    // Create new session
    let session = Session {
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        mode,
        abi_context,
        project_root: project_root.clone(),
        active: true,
    };

    // Save session
    vrift.save_session(&session)?;

    info!(
        "Velo is active in [{}] mode. {}",
        mode,
        match mode {
            ProjectionMode::Solid => "Physical files are safe.",
            ProjectionMode::Phantom => "Project is now purely virtual.",
        }
    );

    Ok(session)
}

/// Deactivate Velo projection
pub fn deactivate(project_root: &Path) -> Result<()> {
    let vrift = VriftDir::new(project_root);

    if !vrift.has_session() {
        info!("No active Velo session found");
        return Ok(());
    }

    let mut session = vrift.load_session()?;
    session.active = false;
    vrift.save_session(&session)?;

    info!("Velo projection deactivated");
    Ok(())
}

/// Validation result for a single projection entry
#[allow(dead_code)]
#[derive(Debug)]
pub struct ValidationResult {
    pub path: PathBuf,
    pub tier: ProjectionTier,
    pub status: ValidationStatus,
}

/// Projection tier classification
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionTier {
    /// Tier-1: Immutable assets (symlink to CAS)
    Tier1,
    /// Tier-2: Mutable assets (hardlink to CAS)
    Tier2,
}

/// Validation status for a projection entry
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationStatus {
    /// Projection is valid
    Valid,
    /// Symlink target missing (Tier-1)
    BrokenSymlink(String),
    /// Hardlink inode mismatch (Tier-2)
    InodeMismatch { expected: u64, actual: u64 },
    /// File missing entirely
    Missing,
    /// File exists but not a projection (regular file)
    NotProjected,
}

/// Startup recovery from session.json and manifest.lmdb (RFC-0039)
///
/// Called on `vrift active` to recover from crashes or unclean shutdowns.
/// Validates existing projections and repairs if necessary.
#[allow(dead_code)]
pub fn startup_recovery(project_root: &Path, cas_root: &Path) -> Result<RecoveryReport> {
    let vrift = VriftDir::new(project_root);

    if !vrift.has_session() {
        debug!("No session found, nothing to recover");
        return Ok(RecoveryReport::default());
    }

    let session = vrift.load_session()?;
    info!(
        mode = %session.mode,
        created = session.created_at,
        "Recovering session"
    );

    // Load manifest if exists
    let manifest_path = vrift.manifest_path();
    if !manifest_path.exists() {
        debug!("No manifest found, skipping validation");
        return Ok(RecoveryReport {
            session_found: true,
            manifest_loaded: false,
            ..Default::default()
        });
    }

    // Validate projections
    let validation_results = validate_projections(project_root, cas_root, &manifest_path)?;

    let mut report = RecoveryReport {
        session_found: true,
        manifest_loaded: true,
        total_entries: validation_results.len(),
        ..Default::default()
    };

    for result in &validation_results {
        match &result.status {
            ValidationStatus::Valid => report.valid_entries += 1,
            ValidationStatus::BrokenSymlink(_) => report.broken_symlinks += 1,
            ValidationStatus::InodeMismatch { .. } => report.inode_mismatches += 1,
            ValidationStatus::Missing => report.missing_entries += 1,
            ValidationStatus::NotProjected => report.not_projected += 1,
        }
    }

    // Auto-repair if needed
    if report.needs_repair() {
        info!(
            broken = report.broken_symlinks,
            mismatched = report.inode_mismatches,
            "Attempting auto-repair"
        );
        let repaired = auto_repair(&validation_results, cas_root)?;
        report.repaired_entries = repaired;
    }

    Ok(report)
}

/// Recovery report from startup_recovery
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct RecoveryReport {
    pub session_found: bool,
    pub manifest_loaded: bool,
    pub total_entries: usize,
    pub valid_entries: usize,
    pub broken_symlinks: usize,
    pub inode_mismatches: usize,
    pub missing_entries: usize,
    pub not_projected: usize,
    pub repaired_entries: usize,
}

#[allow(dead_code)]
impl RecoveryReport {
    /// Check if repairs are needed
    pub fn needs_repair(&self) -> bool {
        self.broken_symlinks > 0 || self.inode_mismatches > 0
    }

    /// Check if all entries are valid
    pub fn all_valid(&self) -> bool {
        self.valid_entries == self.total_entries
    }
}

/// Validate all projections in the manifest
fn validate_projections(
    _project_root: &Path,
    cas_root: &Path,
    _manifest_path: &Path,
) -> Result<Vec<ValidationResult>> {
    // Note: Full implementation would iterate manifest entries
    // For now, we validate the CAS directory structure
    let results = Vec::new();

    // Check CAS root exists
    if !cas_root.exists() {
        debug!(path = %cas_root.display(), "CAS root does not exist");
        return Ok(results);
    }

    // In full implementation:
    // 1. Open LMDB manifest
    // 2. Iterate all entries
    // 3. For each entry, check if projection is valid:
    //    - Tier-1: verify symlink points to correct CAS blob
    //    - Tier-2: verify hardlink inode matches CAS blob

    debug!("Projection validation complete");
    Ok(results)
}

/// Auto-repair broken projections
fn auto_repair(results: &[ValidationResult], cas_root: &Path) -> Result<usize> {
    let mut repaired = 0;

    for result in results {
        if result.status == ValidationStatus::Valid {
            continue;
        }

        match &result.status {
            ValidationStatus::BrokenSymlink(expected_hash) => {
                // Re-create symlink to CAS blob
                let cas_blob = cas_root
                    .join("blake3")
                    .join(&expected_hash[..2])
                    .join(&expected_hash[2..4])
                    .join(expected_hash);

                if cas_blob.exists() {
                    // Remove broken symlink and recreate
                    let _ = fs::remove_file(&result.path);
                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(&cas_blob, &result.path)?;
                        repaired += 1;
                        info!(path = %result.path.display(), "Repaired Tier-1 symlink");
                    }
                }
            }
            ValidationStatus::InodeMismatch { .. } => {
                // For Tier-2, we'd need to recreate the hardlink
                // This is more complex and may require manifest data
                debug!(path = %result.path.display(), "Tier-2 repair not yet implemented");
            }
            _ => {}
        }
    }

    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_vrift_dir_structure() {
        let temp = TempDir::new().unwrap();
        let vrift = VriftDir::new(temp.path());

        vrift.ensure().unwrap();

        assert!(vrift.root.exists());
        assert!(vrift.manifest_path().exists());
    }

    #[test]
    fn test_session_save_load() {
        let temp = TempDir::new().unwrap();
        let vrift = VriftDir::new(temp.path());
        vrift.ensure().unwrap();

        let session = Session {
            created_at: 1706448000,
            mode: ProjectionMode::Solid,
            abi_context: AbiContext::default(),
            project_root: temp.path().to_path_buf(),
            active: true,
        };

        vrift.save_session(&session).unwrap();
        let loaded = vrift.load_session().unwrap();

        assert_eq!(loaded.mode, ProjectionMode::Solid);
        assert!(loaded.active);
    }

    #[test]
    fn test_projection_mode_display() {
        assert_eq!(format!("{}", ProjectionMode::Solid), "Solid");
        assert_eq!(format!("{}", ProjectionMode::Phantom), "Phantom");
    }

    // =========================================================================
    // RFC-0039 Specific Tests
    // =========================================================================

    #[test]
    fn test_activate_creates_session() {
        let temp = TempDir::new().unwrap();

        let session = activate(temp.path(), ProjectionMode::Solid).unwrap();

        assert!(session.active);
        assert_eq!(session.mode, ProjectionMode::Solid);
        assert_eq!(
            session.project_root.canonicalize().unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_activate_phantom_mode() {
        let temp = TempDir::new().unwrap();

        let session = activate(temp.path(), ProjectionMode::Phantom).unwrap();

        assert_eq!(session.mode, ProjectionMode::Phantom);
    }

    #[test]
    fn test_deactivate_sets_inactive() {
        let temp = TempDir::new().unwrap();

        // First activate
        activate(temp.path(), ProjectionMode::Solid).unwrap();

        // Then deactivate
        deactivate(temp.path()).unwrap();

        // Verify session is inactive
        let vrift = VriftDir::new(temp.path());
        let session = vrift.load_session().unwrap();
        assert!(!session.active);
    }

    #[test]
    fn test_startup_recovery_no_session() {
        let temp = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        // No session exists - recovery should return default
        let report = startup_recovery(temp.path(), cas_dir.path()).unwrap();

        assert!(!report.session_found);
        assert!(!report.manifest_loaded);
        assert_eq!(report.total_entries, 0);
    }

    #[test]
    fn test_startup_recovery_with_session() {
        let temp = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        // Create a session first
        activate(temp.path(), ProjectionMode::Solid).unwrap();

        // Run recovery
        let report = startup_recovery(temp.path(), cas_dir.path()).unwrap();

        assert!(report.session_found);
        // Manifest may or may not be loaded depending on if LMDB was created
    }

    #[test]
    fn test_recovery_report_needs_repair() {
        let mut report = RecoveryReport::default();

        assert!(
            !report.needs_repair(),
            "Empty report should not need repair"
        );

        report.broken_symlinks = 1;
        assert!(
            report.needs_repair(),
            "Report with broken symlinks needs repair"
        );

        report.broken_symlinks = 0;
        report.inode_mismatches = 1;
        assert!(
            report.needs_repair(),
            "Report with inode mismatches needs repair"
        );
    }

    #[test]
    fn test_recovery_report_all_valid() {
        let mut report = RecoveryReport {
            total_entries: 5,
            valid_entries: 5,
            ..Default::default()
        };

        assert!(report.all_valid(), "All entries should be valid");

        report.valid_entries = 3;
        assert!(!report.all_valid(), "Not all entries are valid");
    }

    #[test]
    fn test_validation_status_equality() {
        assert_eq!(ValidationStatus::Valid, ValidationStatus::Valid);
        assert_eq!(ValidationStatus::Missing, ValidationStatus::Missing);
        assert_ne!(ValidationStatus::Valid, ValidationStatus::Missing);
    }

    #[test]
    fn test_projection_tier_classification() {
        assert_eq!(ProjectionTier::Tier1, ProjectionTier::Tier1);
        assert_ne!(ProjectionTier::Tier1, ProjectionTier::Tier2);
    }

    #[test]
    fn test_abi_context_detection() {
        let ctx = detect_abi_context();

        // Should at least have target triple
        assert!(!ctx.target_triple.is_empty());
        // Platform should be part of target
        assert!(ctx.target_triple.contains(std::env::consts::OS));
    }
}
