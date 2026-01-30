//! Zero-Copy Ingest Pipeline (RFC-0039 Aligned)
//!
//! Per RFC-0039, ingest uses O(1) filesystem operations:
//! - Solid Mode: hard_link() + symlink replacement
//! - Phantom Mode: rename() (atomic move)
//!
//! NO data copying - only metadata operations.

use std::fs::{self, File};
use std::os::unix::fs as unix_fs;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nix::fcntl::{flock, FlockArg};

use crate::{Blake3Hash, CasError, Result};

// ============================================================================
// Configuration
// ============================================================================

#[derive(Clone, Debug)]
pub struct ZeroCopyConfig {
    /// Channel capacity (default: 1024)
    pub channel_capacity: usize,
    /// Number of worker threads (default: num_cpus)
    pub worker_threads: usize,
}

impl Default for ZeroCopyConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1024,
            worker_threads: num_cpus::get(),
        }
    }
}

// ============================================================================
// Ingest Result
// ============================================================================

#[derive(Debug)]
pub struct IngestResult {
    pub source_path: PathBuf,
    pub hash: Blake3Hash,
    pub size: u64,
}

// ============================================================================
// Zero-Copy Ingest Functions
// ============================================================================

/// Ingest Solid Mode Tier-1 (Immutable): hard_link + symlink
///
/// 1. Acquire flock(LOCK_SH) to block external writers
/// 2. Stream hash the file (no full read into memory)
/// 3. Create hard link to CAS (zero-copy!)
/// 4. Replace source with symlink
pub fn ingest_solid_tier1(source: &Path, cas_root: &Path) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    
    // Acquire shared lock (blocks external writers)
    flock(file.as_raw_fd(), FlockArg::LockShared)
        .map_err(|e| CasError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    
    // Stream hash (no full read)
    let hash = stream_hash(&file)?;
    let cas_target = cas_path(cas_root, &hash, size);
    
    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Hard link (zero-copy!)
    if !cas_target.exists() {
        fs::hard_link(source, &cas_target)?;
    }
    
    // Unlock before modifying source
    flock(file.as_raw_fd(), FlockArg::Unlock)
        .map_err(|e| CasError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    drop(file);
    
    // Replace source with symlink
    fs::remove_file(source)?;
    unix_fs::symlink(&cas_target, source)?;
    
    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
    })
}

/// Ingest Solid Mode Tier-2 (Mutable): hard_link only (keep original)
pub fn ingest_solid_tier2(source: &Path, cas_root: &Path) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    
    // Acquire shared lock
    flock(file.as_raw_fd(), FlockArg::LockShared)
        .map_err(|e| CasError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    
    // Stream hash
    let hash = stream_hash(&file)?;
    let cas_target = cas_path(cas_root, &hash, size);
    
    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Hard link (zero-copy!)
    if !cas_target.exists() {
        fs::hard_link(source, &cas_target)?;
    }
    
    flock(file.as_raw_fd(), FlockArg::Unlock)
        .map_err(|e| CasError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    
    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
    })
}

/// Ingest Phantom Mode: atomic rename (file moves to CAS)
pub fn ingest_phantom(source: &Path, cas_root: &Path) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    
    // Acquire shared lock
    flock(file.as_raw_fd(), FlockArg::LockShared)
        .map_err(|e| CasError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    
    // Stream hash
    let hash = stream_hash(&file)?;
    let cas_target = cas_path(cas_root, &hash, size);
    
    // Unlock and close before rename
    flock(file.as_raw_fd(), FlockArg::Unlock)
        .map_err(|e| CasError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    drop(file);
    
    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Atomic move (zero-copy!)
    if !cas_target.exists() {
        fs::rename(source, &cas_target)?;
    } else {
        // Already in CAS, just delete source
        fs::remove_file(source)?;
    }
    
    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Stream hash using mmap (no full read into memory)
fn stream_hash(file: &File) -> Result<Blake3Hash> {
    let mmap = unsafe { memmap2::Mmap::map(file)? };
    let hash = blake3::hash(&mmap);
    Ok(*hash.as_bytes())
}

/// 3-level sharded CAS path: blake3/ab/cd/hash_size.bin
fn cas_path(cas_root: &Path, hash: &Blake3Hash, size: u64) -> PathBuf {
    let hex = hex::encode(hash);
    cas_root
        .join("blake3")
        .join(&hex[0..2])
        .join(&hex[2..4])
        .join(format!("{}_{}.bin", hex, size))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::io::Write;

    fn setup() -> (TempDir, TempDir, PathBuf) {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();
        
        let test_file = source_dir.path().join("test.txt");
        let mut f = File::create(&test_file).unwrap();
        f.write_all(b"Hello, zero-copy!").unwrap();
        
        (source_dir, cas_dir, test_file)
    }

    #[test]
    fn test_solid_tier2_zero_copy() {
        let (source_dir, cas_dir, test_file) = setup();
        
        let result = ingest_solid_tier2(&test_file, cas_dir.path()).unwrap();
        
        // Original still exists (Tier-2 keeps it)
        assert!(test_file.exists());
        
        // CAS has the file
        let cas_file = cas_path(cas_dir.path(), &result.hash, result.size);
        assert!(cas_file.exists());
        
        // Same content
        assert_eq!(fs::read(&test_file).unwrap(), fs::read(&cas_file).unwrap());
    }

    #[test]
    fn test_phantom_zero_copy() {
        let (source_dir, cas_dir, test_file) = setup();
        let original_content = fs::read(&test_file).unwrap();
        
        let result = ingest_phantom(&test_file, cas_dir.path()).unwrap();
        
        // Original is gone (moved)
        assert!(!test_file.exists());
        
        // CAS has the file
        let cas_file = cas_path(cas_dir.path(), &result.hash, result.size);
        assert!(cas_file.exists());
        assert_eq!(fs::read(&cas_file).unwrap(), original_content);
    }
}
