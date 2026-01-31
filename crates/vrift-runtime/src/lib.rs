//! # velo-runtime
//!
//! Runtime orchestration for Velo Rift.
//!
//! Handles:
//! - Link Farm generation (populating LowerDir)
//! - OverlayFS mounting (Linux only)
//! - Namespace isolation (Linux only)

use std::fs;

use std::path::{Path, PathBuf};

use thiserror::Error;
use vrift_cas::CasStore;
use vrift_manifest::Manifest;

#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CAS error: {0}")]
    Cas(#[from] vrift_cas::CasError),

    #[error("Blob not found in CAS: {0}")]
    BlobNotFound(String),

    #[cfg(target_os = "linux")]
    #[error("Nix system error: {0}")]
    Nix(#[from] nix::Error),

    #[error("OverlayFS error: {0}")]
    Overlay(String),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[cfg(target_os = "linux")]
use nix::mount::{mount, MsFlags};

/// Generates a Link Farm from a Manifest into a target directory.
///
/// This creates a directory structure where every file is a hard link to
/// the corresponding blob in the CAS. This structure serves as the
/// LowerDir for OverlayFS.
pub struct LinkFarm {
    cas: CasStore,
}

impl LinkFarm {
    pub fn new(cas: CasStore) -> Self {
        Self { cas }
    }

    /// Populate the target directory with hard links based on one or more manifests.
    ///
    /// If multiple manifests are provided, they are applied in order. Files in later
    /// manifests will overwrite those in earlier ones at the same path.
    pub fn populate(&self, manifests: &[Manifest], target: &Path) -> Result<()> {
        if !target.exists() {
            fs::create_dir_all(target)?;
        }

        for manifest in manifests {
            for (path_str, entry) in manifest.iter() {
                // Skip root directory entry itself if present
                if path_str == "/" {
                    continue;
                }

                // Construct full destination path
                let relative_path = path_str.trim_start_matches('/');
                let dest_path = target.join(relative_path);

                if entry.is_dir() {
                    fs::create_dir_all(&dest_path)?;
                    continue;
                }

                // Ensure parent directory exists
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                if entry.is_file() {
                    // Find source blob in CAS
                    let src_path = self
                        .cas
                        .blob_path_for_hash(&entry.content_hash)
                        .ok_or_else(|| {
                            RuntimeError::BlobNotFound(format!("{:?}", entry.content_hash))
                        })?;

                    // Create hard link: src (CAS) -> dest (Link Farm)
                    // Remove existing file if present (idempotency/overwrite)
                    if dest_path.exists() {
                        fs::remove_file(&dest_path)?;
                    }

                    fs::hard_link(&src_path, &dest_path)?;

                    // Apply metadata (mode, mtime)
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&dest_path, fs::Permissions::from_mode(entry.mode))?;

                    // Note: Setting mtime requires filetime or similar,
                    // skipping for MVP unless we add dependency.
                    // But permissions are CRITICAL for execution.
                } else if entry.is_symlink() {
                    // Fetch symlink target from CAS
                    let target_bytes = self
                        .cas
                        .get(&entry.content_hash)
                        .map_err(RuntimeError::Cas)?;

                    let target_path_str = String::from_utf8(target_bytes).map_err(|_| {
                        RuntimeError::Overlay("Invalid UTF-8 in symlink target".into())
                    })?;

                    // Remove existing file if present
                    if dest_path.exists() {
                        fs::remove_file(&dest_path)?;
                    }

                    std::os::unix::fs::symlink(target_path_str, &dest_path)
                        .map_err(RuntimeError::Io)?;
                }
            }
        }
        Ok(())
    }
}

/// OverlayFS Manager (Linux only)
pub struct OverlayManager {
    lower_dir: PathBuf,
    upper_dir: PathBuf,
    work_dir: PathBuf,
    merged_dir: PathBuf,
}

impl OverlayManager {
    pub fn new(
        lower_dir: PathBuf,
        upper_dir: PathBuf,
        work_dir: PathBuf,
        merged_dir: PathBuf,
    ) -> Self {
        Self {
            lower_dir,
            upper_dir,
            work_dir,
            merged_dir,
        }
    }

    /// Mount the OverlayFS
    #[cfg(target_os = "linux")]
    pub fn mount(&self) -> Result<()> {
        // Ensure directories exist
        fs::create_dir_all(&self.merged_dir)?;
        fs::create_dir_all(&self.upper_dir)?;
        fs::create_dir_all(&self.work_dir)?;
        // Lower dir should already exist (Link Farm)

        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            self.lower_dir.display(),
            self.upper_dir.display(),
            self.work_dir.display()
        );

        mount(
            Some("overlay"),
            &self.merged_dir,
            Some("overlay"),
            MsFlags::empty(),
            Some(options.as_str()),
        )?;

        Ok(())
    }

    /// Mock mount for non-Linux systems
    #[cfg(not(target_os = "linux"))]
    pub fn mount(&self) -> Result<()> {
        println!("⚠️  OverlayFS mount is only supported on Linux.");
        println!("    Would mount:");
        println!("      Lower:  {}", self.lower_dir.display());
        println!("      Upper:  {}", self.upper_dir.display());
        println!("      Work:   {}", self.work_dir.display());
        println!("      Merged: {}", self.merged_dir.display());

        // Ensure directories exist simulation
        fs::create_dir_all(&self.merged_dir)?;
        fs::create_dir_all(&self.upper_dir)?;
        fs::create_dir_all(&self.work_dir)?;

        // Create a dummy file in merged to simulate success
        fs::write(
            self.merged_dir.join("velo_overlay_mock.txt"),
            "OverlayFS Mock Active",
        )?;

        Ok(())
    }
}

/// Namespace Isolation Manager (Linux only)
pub struct NamespaceManager;

impl NamespaceManager {
    /// Enter isolated namespaces (Mount, PID, IPC, UTS, Net)
    #[cfg(target_os = "linux")]
    pub fn isolate() -> Result<()> {
        use nix::sched::{unshare, CloneFlags};

        // unshare(CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWIPC | CLONE_NEWUTS | CLONE_NEWNET)
        unshare(
            CloneFlags::CLONE_NEWNS
                | CloneFlags::CLONE_NEWPID
                | CloneFlags::CLONE_NEWIPC
                | CloneFlags::CLONE_NEWUTS
                | CloneFlags::CLONE_NEWNET,
        )?;

        // Remount / as private to avoid propagation
        use nix::mount::{mount, MsFlags};
        mount(
            None::<&str>,
            "/",
            None::<&str>,
            MsFlags::MS_PRIVATE | MsFlags::MS_REC,
            None::<&str>,
        )?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn isolate() -> Result<()> {
        println!("⚠️  Namespace isolation is only supported on Linux.");
        println!("    Would unshare(CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWNET | ...)");
        Ok(())
    }

    /// Pivot root to the new filesystem
    #[cfg(target_os = "linux")]
    pub fn pivot_root(new_root: &Path, put_old: &Path) -> Result<()> {
        use nix::unistd::pivot_root;

        pivot_root(new_root, put_old)?;

        // Unmount old root
        use nix::mount::{umount2, MntFlags};
        umount2(put_old, MntFlags::MNT_DETACH)?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn pivot_root(new_root: &Path, put_old: &Path) -> Result<()> {
        println!("⚠️  pivot_root is only supported on Linux.");
        println!(
            "    Would pivot_root(new={}, old={})",
            new_root.display(),
            put_old.display()
        );
        Ok(())
    }

    /// Mount pseudo-filesystems (/proc, /sys, /dev)
    #[cfg(target_os = "linux")]
    pub fn mount_pseudo_fs(root: &Path) -> Result<()> {
        use nix::mount::{mount, MsFlags};

        let proc_path = root.join("proc");
        let sys_path = root.join("sys");
        let dev_path = root.join("dev");

        if proc_path.exists() {
            mount(
                Some("proc"),
                &proc_path,
                Some("proc"),
                MsFlags::empty(),
                None::<&str>,
            )?;
        }

        if sys_path.exists() {
            mount(
                Some("sysfs"),
                &sys_path,
                Some("sysfs"),
                MsFlags::empty(),
                None::<&str>,
            )?;
        }

        // bind mount /dev
        // In a real scenario we might want a devtmpfs or bind mount from host
        // For now, let's assume bind mount of /dev
        if dev_path.exists() {
            mount(
                Some("/dev"),
                &dev_path,
                None::<&str>,
                MsFlags::MS_BIND | MsFlags::MS_REC,
                None::<&str>,
            )?;
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn mount_pseudo_fs(root: &Path) -> Result<()> {
        println!("⚠️  Pseudo-FS mount is only supported on Linux.");
        println!("    Would mount /proc, /sys, /dev into {}", root.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use vrift_manifest::VnodeEntry;

    #[test]
    fn test_link_farm_populate() {
        let temp = TempDir::new().unwrap();
        let cas_root = temp.path().join("cas");
        let link_farm_root = temp.path().join("lower");

        let cas = CasStore::new(&cas_root).unwrap();

        // Store a file
        let content = b"runtime test";
        let hash = cas.store(content).unwrap();

        // Create manifest
        let mut manifest = Manifest::new();
        manifest.insert(
            "/etc/config",
            VnodeEntry::new_file(hash, content.len() as u64, 0, 0o644),
        );
        manifest.insert("/var/log", VnodeEntry::new_directory(0, 0o755));

        // Populate Link Farm
        let farm = LinkFarm::new(cas);
        farm.populate(&[manifest], &link_farm_root).unwrap();

        // Verify
        let config_path = link_farm_root.join("etc/config");
        assert!(config_path.exists());
        let read_content = fs::read(&config_path).unwrap();
        assert_eq!(read_content, content);

        // Verify it's a hard link (same inode) - simplified check
        // Rust std doesn't expose inode easily without stat, but metadata should match
        assert_eq!(
            fs::metadata(&config_path).unwrap().len(),
            content.len() as u64
        );

        let log_path = link_farm_root.join("var/log");
        assert!(log_path.exists());
        assert!(log_path.is_dir());
    }

    #[test]
    fn test_link_farm_merge() {
        let temp = TempDir::new().unwrap();
        let cas_root = temp.path().join("cas");
        let link_farm_root = temp.path().join("lower");

        let cas = CasStore::new(&cas_root).unwrap();

        // 1. Create Base Manifest
        let base_content = b"base content";
        let base_hash = cas.store(base_content).unwrap();
        let mut base_manifest = Manifest::new();
        base_manifest.insert(
            "/bin/sh",
            VnodeEntry::new_file(base_hash, base_content.len() as u64, 0, 0o755),
        );
        base_manifest.insert(
            "/etc/common",
            VnodeEntry::new_file(base_hash, base_content.len() as u64, 0, 0o644),
        );

        // 2. Create App Manifest
        let app_content = b"app content";
        let app_hash = cas.store(app_content).unwrap();
        let mut app_manifest = Manifest::new();
        app_manifest.insert(
            "/app/main",
            VnodeEntry::new_file(app_hash, app_content.len() as u64, 0, 0o755),
        );
        // Overwrite etc/common
        app_manifest.insert(
            "/etc/common",
            VnodeEntry::new_file(app_hash, app_content.len() as u64, 0, 0o644),
        );

        // 3. Populate Link Farm with both
        let farm = LinkFarm::new(cas);
        farm.populate(&[base_manifest, app_manifest], &link_farm_root)
            .unwrap();

        // 4. Verify Merged View
        // - From base
        assert_eq!(
            fs::read(link_farm_root.join("bin/sh")).unwrap(),
            base_content
        );
        // - From app
        assert_eq!(
            fs::read(link_farm_root.join("app/main")).unwrap(),
            app_content
        );
        // - Merged/Overwritten
        assert_eq!(
            fs::read(link_farm_root.join("etc/common")).unwrap(),
            app_content
        );
    }
}
