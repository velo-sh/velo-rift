//! Active projection mode implementation for Velo Rift™
//!
//! Implements `vrift active` command for RFC-0039 Transparent Virtual Projection.
//! Creates a persistent session that projects dependencies from CAS.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

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
        fs::create_dir_all(self.manifest_path())
            .with_context(|| format!("Failed to create manifest directory"))?;
        
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
        let session: Session = serde_json::from_str(&content)
            .with_context(|| "Failed to parse session.json")?;
        Ok(session)
    }
    
    /// Save session
    pub fn save_session(&self, session: &Session) -> Result<()> {
        let content = serde_json::to_string_pretty(session)
            .with_context(|| "Failed to serialize session")?;
        fs::write(self.session_path(), content)
            .with_context(|| "Failed to write session.json")?;
        Ok(())
    }
}

/// Detect ABI context from the current environment
pub fn detect_abi_context() -> AbiContext {
    let target_triple = std::env::consts::ARCH.to_string()
        + "-"
        + std::env::consts::OS;
    
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
    let project_root = project_root.canonicalize()
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
    
    info!("Velo is active in [{}] mode. {}", 
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
}
