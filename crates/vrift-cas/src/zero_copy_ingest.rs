//! Zero-Copy Ingest Pipeline (RFC-0039 Aligned)
//!
//! Per RFC-0039, ingest uses O(1) filesystem operations:
//! - Solid Mode: hard_link() + symlink replacement
//! - Phantom Mode: rename() (atomic move)
//!
//! NO data copying - only metadata operations.
//!
//! # Parallel Deduplication
//!
//! When processing files in parallel, uses DashSet for in-memory dedup
//! to skip filesystem writes for already-seen hashes.
//!
//! # Tiered Fallback Strategy (Pattern 987)
//!
//! For macOS code-signed bundles (.app, .framework), hard_link fails with EPERM.
//! Strategy: hard_link → clonefile (APFS CoW) → copy
//!
//! This provides optimal performance while handling all edge cases.

use std::fs::{self, File};
use std::io;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use dashmap::DashSet;
use nix::fcntl::{Flock, FlockArg};

use crate::{Blake3Hash, CasError, Result};

// ============================================================================
// Tiered Link Strategy: hard_link → clonefile → copy
// ============================================================================

/// Tiered file linking strategy for CAS storage
///
/// Attempts operations in order of efficiency:
/// 1. hard_link() - zero-copy, shares inode
/// 2. clonefile() - zero-copy on APFS, separate inode (works with code-signed)
/// 3. copy() - full copy fallback
///
/// # Arguments
/// * `source` - Source file path
/// * `target` - Target path in CAS
///
/// # Returns
/// * Ok(true) if a new file was created
/// * Ok(false) if file already existed (dedup)
/// * Err on all fallback methods failed
///
/// Tiered file linking strategy for CAS storage
///
/// Refactored to use LinkStrategy for Inode Decoupling (Reflink Priority).
fn link_or_clone_or_copy(source: &Path, target: &Path) -> io::Result<bool> {
    if target.exists() {
        // Idempotency: re-enforce CAS invariant on the TARGET only.
        let _ = crate::protection::enforce_cas_invariant(target);
        return Ok(false);
    }

    // Use platform-optimal LinkStrategy (RFC-0040)
    // Priority: Reflink -> Hardlink -> Copy
    crate::link_strategy::get_strategy().link_file(source, target)?;

    Ok(true)
}

// ============================================================================
// Tier-1 Immutable Flag (RFC-0039 §5.1.1)
// ============================================================================

// RFC-0039 §5.1.1: Set immutable flag for maximum Tier-1 protection moved to crates/vrift-cas/src/protection.rs
use crate::protection::set_immutable as set_immutable_native;

/// Best-effort immutable flag setting.
///
/// Log errors but don't fail ingest if permissions are insufficient (e.g. non-root on Linux).
fn set_immutable_best_effort(path: &std::path::Path) {
    if let Err(e) = set_immutable_native(path, true) {
        tracing::debug!("Failed to set immutable flag on {:?}: {}", path, e);
    }
}

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
    /// True if this was a new blob (not a duplicate)
    pub was_new: bool,
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

    // Acquire shared lock with retry (prevents blocking on busy files)
    let locked_file = lock_with_retry(file, FlockArg::LockShared)?;

    // Tiered hash: read() for small files, mmap for larger
    let hash = tiered_hash(&locked_file, size)?;
    let cas_target = cas_path(cas_root, &hash, size);

    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }

    // Tiered link: hard_link → clonefile → copy (RFC-0040)
    let was_new = link_or_clone_or_copy(source, &cas_target)?;

    // Drop the lock guard before modifying source
    drop(locked_file);

    // Replace source with symlink
    tracing::debug!("Replacing {:?} with symlink to {:?}", source, cas_target);
    fs::remove_file(source)?;
    unix_fs::symlink(&cas_target, source)?;
    tracing::debug!("Symlink created successfully");

    // RFC-0039 Iron Law: ALWAYS enforce CAS invariant, even for existing blobs
    // This fixes the "Iron Law Drift" bug where pre-existing writable blobs
    // would not be protected on re-ingest.
    let _ = crate::protection::enforce_cas_invariant(&cas_target);
    if was_new {
        set_immutable_best_effort(&cas_target);
    }

    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
        was_new,
    })
}

/// Ingest Solid Mode Tier-1 with in-memory deduplication
///
/// Uses DashSet to skip hard_link when CAS blob already exists,
/// but still performs symlink replacement for each source file.
///
/// # Arguments
///
/// * `source` - Path to file to ingest
/// * `cas_root` - CAS storage root  
/// * `seen_hashes` - Concurrent set of already-processed hashes
pub fn ingest_solid_tier1_dedup(
    source: &Path,
    cas_root: &Path,
    seen_hashes: &DashSet<String>,
) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();

    // P3 Optimization: Try optimistic hash (no flock) for small read-only files
    let hash = if let Some(h) = optimistic_hash_with_validation(&file, &metadata)? {
        h
    } else {
        // Standard path: acquire flock for larger or writable files
        let locked_file = lock_with_retry(file, FlockArg::LockShared)?;
        tiered_hash(&locked_file, size)?
    };
    let hash_key = hex::encode(hash);
    let cas_target = cas_path(cas_root, &hash, size);

    // In-memory dedup: only create hard_link if first time seeing this hash
    let is_new = seen_hashes.insert(hash_key);

    if is_new {
        // Create CAS directory if needed
        if let Some(parent) = cas_target.parent() {
            fs::create_dir_all(parent)?;
        }

        // Tiered link: hard_link → clonefile → copy (RFC-0040)
        link_or_clone_or_copy(source, &cas_target)?;
    }

    // Lock (if acquired) is automatically dropped here

    // Always replace source with symlink (even if CAS blob already existed)
    fs::remove_file(source)?;
    unix_fs::symlink(&cas_target, source)?;

    // RFC-0039 §5.1.1: Set immutable flag for maximum Tier-1 protection (must do after source removal!)
    if is_new {
        // RFC-0039 Iron Law: Ensure CAS blob is read-only and NOT executable
        let _ = crate::protection::enforce_cas_invariant(&cas_target);
        set_immutable_best_effort(&cas_target);
    }

    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
        was_new: is_new, // is_new from insert() tells us if this was first time
    })
}

/// Ingest Solid Mode Tier-2 (Mutable): hard_link only (keep original)
pub fn ingest_solid_tier2(source: &Path, cas_root: &Path) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();

    // P3 Optimization: Try optimistic hash (no flock) for small read-only files
    let hash = if let Some(h) = optimistic_hash_with_validation(&file, &metadata)? {
        h
    } else {
        // Standard path: acquire flock for larger or writable files
        let locked_file = lock_with_retry(file, FlockArg::LockShared)?;
        tiered_hash(&locked_file, size)?
    };

    let cas_target = cas_path(cas_root, &hash, size);

    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }

    // Tiered link: hard_link → clonefile → copy (RFC-0040)
    let was_new = link_or_clone_or_copy(source, &cas_target)?;

    // RFC-0039 Iron Law: ALWAYS enforce CAS invariant, even for existing blobs
    let _ = crate::protection::enforce_cas_invariant(&cas_target);
    if was_new {
        set_immutable_best_effort(&cas_target);
    }

    // Lock guard auto-drops here
    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
        was_new,
    })
}

/// Ingest Solid Mode Tier-2 with in-memory deduplication
///
/// Uses DashSet to track already-seen hashes, skipping filesystem
/// operations entirely for duplicate files. This is more efficient
/// than letting the filesystem reject duplicates with EEXIST.
///
/// # Arguments
///
/// * `source` - Path to file to ingest  
/// * `cas_root` - CAS storage root
/// * `seen_hashes` - Concurrent set of already-processed hashes
///
/// # Returns
///
/// IngestResult with hash and size (filesystem write may be skipped)
pub fn ingest_solid_tier2_dedup(
    source: &Path,
    cas_root: &Path,
    seen_hashes: &DashSet<String>,
) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();

    // P3 Optimization: Try optimistic hash (no flock) for small read-only files
    let hash = if let Some(h) = optimistic_hash_with_validation(&file, &metadata)? {
        h
    } else {
        // Standard path: acquire flock for larger or writable files
        let locked_file = lock_with_retry(file, FlockArg::LockShared)?;
        tiered_hash(&locked_file, size)?
    };
    let hash_key = hex::encode(hash);

    // In-memory dedup: if hash already seen, skip filesystem write
    if !seen_hashes.insert(hash_key) {
        // Already processed by another thread - skip hard_link entirely
        return Ok(IngestResult {
            source_path: source.to_owned(),
            hash,
            size,
            was_new: false, // Duplicate - already processed
        });
    }

    let cas_target = cas_path(cas_root, &hash, size);

    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }

    // Tiered link: hard_link → clonefile → copy (RFC-0040)
    let was_new = link_or_clone_or_copy(source, &cas_target)?;

    // RFC-0039 Iron Law: ALWAYS enforce CAS invariant, even for existing blobs
    let _ = crate::protection::enforce_cas_invariant(&cas_target);
    if was_new {
        set_immutable_best_effort(&cas_target);
    }

    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
        was_new,
    })
}

/// Ingest Phantom Mode: atomic rename (file moves to CAS)
pub fn ingest_phantom(source: &Path, cas_root: &Path) -> Result<IngestResult> {
    let file = File::open(source)?;
    let metadata = file.metadata()?;
    let size = metadata.len();

    // Acquire shared lock with retry (prevents blocking on busy files)
    let locked_file = lock_with_retry(file, FlockArg::LockShared)?;

    // Tiered hash: read() for small files, mmap for larger
    let hash = tiered_hash(&locked_file, size)?;
    let cas_target = cas_path(cas_root, &hash, size);

    // Drop lock guard before rename
    drop(locked_file);

    // Create CAS directory if needed
    if let Some(parent) = cas_target.parent() {
        fs::create_dir_all(parent)?;
    }

    // RFC-0039 Audit: If target already exists and is immutable, rename will fail with EPERM.
    // We check existence first to handle duplicate ingest safely.
    if cas_target.exists() {
        let _ = fs::remove_file(source); // Clean up source since it's already in CAS
        return Ok(IngestResult {
            source_path: source.to_owned(),
            hash,
            size,
            was_new: false,
        });
    }

    // Atomic move (zero-copy!) - handle race condition
    match fs::rename(source, &cas_target) {
        Ok(()) => {
            // RFC-0039 Iron Law: Ensure CAS blob is read-only and NOT executable
            let _ = crate::protection::enforce_cas_invariant(&cas_target);
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another thread already created this blob, delete source
            let _ = fs::remove_file(source);
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            // Likely existing immutable blob (race condition)
            if cas_target.exists() {
                let _ = fs::remove_file(source);
                return Ok(IngestResult {
                    source_path: source.to_owned(),
                    hash,
                    size,
                    was_new: false,
                });
            }
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    }

    Ok(IngestResult {
        source_path: source.to_owned(),
        hash,
        size,
        was_new: true, // Phantom always creates new (rename)
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// P3 Optimization: Skip flock for small, read-only files
/// Small files are unlikely to be modified during ingest
const SKIP_FLOCK_SIZE_THRESHOLD: u64 = 4096; // 4KB

/// Optimistic hash without flock for small read-only files.
///
/// Validation: Compare mtime+size before and after hash to detect concurrent modification.
/// If mtime/size changed during hash, this is a concurrent write and we should retry with flock.
fn optimistic_hash_with_validation(
    file: &File,
    initial_metadata: &std::fs::Metadata,
) -> Result<Option<Blake3Hash>> {
    let size = initial_metadata.len();

    // Only skip flock for small, read-only files
    if size >= SKIP_FLOCK_SIZE_THRESHOLD || !initial_metadata.permissions().readonly() {
        return Ok(None); // Caller should use flock path
    }

    // Hash without flock
    let hash = tiered_hash(file, size)?;

    // Validate: check if file was modified during hash
    if let Ok(post_metadata) = file.metadata() {
        let post_mtime = post_metadata.modified().ok();
        let pre_mtime = initial_metadata.modified().ok();

        if post_metadata.len() == size && post_mtime == pre_mtime {
            return Ok(Some(hash)); // File unchanged, hash is valid
        }
    }

    // File was modified, caller should retry with flock
    Ok(None)
}

/// Lock retry configuration
const MAX_LOCK_RETRIES: u32 = 5;
const INITIAL_RETRY_DELAY_MS: u64 = 10;

/// Acquire flock with retry and exponential backoff
///
/// Prevents blocking when files are temporarily locked by other processes.
/// Uses non-blocking lock attempts with exponential backoff delays.
fn lock_with_retry(mut file: File, lock_type: FlockArg) -> Result<Flock<File>> {
    let mut delay_ms = INITIAL_RETRY_DELAY_MS;

    for attempt in 0..MAX_LOCK_RETRIES {
        // Try non-blocking lock first
        let lock_arg = match lock_type {
            FlockArg::LockShared => FlockArg::LockSharedNonblock,
            FlockArg::LockExclusive => FlockArg::LockExclusiveNonblock,
            other => other,
        };

        match Flock::lock(file, lock_arg) {
            Ok(guard) => return Ok(guard),
            Err((returned_file, err)) => {
                // Check if it's EWOULDBLOCK (lock unavailable)
                if err == nix::errno::Errno::EWOULDBLOCK && attempt < MAX_LOCK_RETRIES - 1 {
                    // Wait with exponential backoff before retry
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    delay_ms *= 2;
                    file = returned_file;
                    continue;
                }

                // Last attempt: try blocking lock
                if attempt == MAX_LOCK_RETRIES - 1 {
                    return Flock::lock(returned_file, lock_type).map_err(|(_, e)| {
                        CasError::Io(std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            format!(
                                "Failed to acquire lock after {} retries: {}",
                                MAX_LOCK_RETRIES, e
                            ),
                        ))
                    });
                }

                return Err(CasError::Io(std::io::Error::other(err.to_string())));
            }
        }
    }

    unreachable!()
}

/// Tiered hashing strategy for optimal performance:
/// - Small files (< 16KB): Direct read() avoids mmap setup overhead (~10µs)
/// - Medium/Large files (>= 16KB): mmap for zero-copy access
const SMALL_FILE_THRESHOLD: u64 = 16 * 1024; // 16KB

fn tiered_hash(file: &File, size: u64) -> Result<Blake3Hash> {
    if size < SMALL_FILE_THRESHOLD {
        // Small file: direct read avoids mmap syscall overhead
        let mut buf = vec![0u8; size as usize];
        use std::io::Read;
        (&*file).read_exact(&mut buf)?;
        Ok(*blake3::hash(&buf).as_bytes())
    } else {
        // Medium/Large file: mmap for zero-copy
        // SAFETY: mmap requires a valid file descriptor
        let mmap = unsafe { memmap2::Mmap::map(file)? };
        Ok(*blake3::hash(&mmap).as_bytes())
    }
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
    use std::io::Write;
    use tempfile::TempDir;

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
        let (_source_dir, cas_dir, test_file) = setup();

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
        let (_source_dir, cas_dir, test_file) = setup();
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
