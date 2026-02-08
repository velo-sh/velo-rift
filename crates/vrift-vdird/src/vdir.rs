//! VDir mmap management

use anyhow::{Context, Result};
use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, info, warn};

/// VDir constants
pub const VDIR_MAGIC: u32 = 0x56524654; // "VRFT"
pub const VDIR_VERSION: u32 = 2; // v2: Added CRC32 checksum
pub const VDIR_DEFAULT_CAPACITY: usize = 65536;
pub const VDIR_ENTRY_SIZE: usize = std::mem::size_of::<VDirEntry>();
pub const VDIR_HEADER_SIZE: usize = std::mem::size_of::<VDirHeader>();

/// VDir header in shared memory
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VDirHeader {
    pub magic: u32,
    pub version: u32,
    pub generation: u64, // Atomic counter for synchronization
    pub entry_count: u32,
    pub table_capacity: u32,
    pub table_offset: u32,
    pub crc32: u32,     // CRC32 checksum of header (computed over fields before crc32)
    pub _pad: [u8; 32], // Pad to 64 bytes (reduced from 36 to 32)
}

/// Single VDir entry
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VDirEntry {
    pub path_hash: u64,     // FNV-1a hash of path
    pub cas_hash: [u8; 32], // BLAKE3 content hash
    pub size: u64,
    pub mtime_sec: i64,
    pub mtime_nsec: u32,
    pub mode: u32,
    pub flags: u16, // DIRTY, DELETED, SYMLINK, DIR
    pub _pad: [u16; 3],
}

/// Flag definitions
pub const FLAG_DIRTY: u16 = 0x0001;
pub const FLAG_DELETED: u16 = 0x0002;
pub const FLAG_SYMLINK: u16 = 0x0004;
pub const FLAG_DIR: u16 = 0x0008;

impl VDirEntry {
    pub fn is_empty(&self) -> bool {
        self.path_hash == 0
    }

    pub fn is_dirty(&self) -> bool {
        (self.flags & FLAG_DIRTY) != 0
    }

    pub fn is_dir(&self) -> bool {
        (self.flags & FLAG_DIR) != 0
    }
}

/// VDir manager
pub struct VDir {
    mmap: MmapMut,
    capacity: usize,
}

impl VDir {
    /// Create or open existing VDir mmap file
    pub fn create_or_open(path: &Path) -> Result<Self> {
        let capacity = VDIR_DEFAULT_CAPACITY;
        let file_size = VDIR_HEADER_SIZE + (capacity * VDIR_ENTRY_SIZE);

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .context("Failed to open VDir file")?;

        // Set file size if new
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            file.set_len(file_size as u64)?;
            info!(path = %path.display(), size = file_size, "Created new VDir file");
        }

        // mmap the file
        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        // Initialize or validate header
        let header = unsafe { &mut *(mmap.as_mut_ptr() as *mut VDirHeader) };
        let needs_init = header.magic != VDIR_MAGIC || header.version != VDIR_VERSION;

        if needs_init {
            if header.magic == VDIR_MAGIC && header.version < VDIR_VERSION {
                info!(
                    old_version = header.version,
                    new_version = VDIR_VERSION,
                    "Upgrading VDir version"
                );
            }
            *header = VDirHeader {
                magic: VDIR_MAGIC,
                version: VDIR_VERSION,
                generation: 0,
                entry_count: 0,
                table_capacity: capacity as u32,
                table_offset: VDIR_HEADER_SIZE as u32,
                crc32: 0,
                _pad: [0; 32],
            };
            header.crc32 = Self::compute_header_crc(header);
            mmap.flush()?;
            debug!("Initialized VDir header");
        } else {
            // Validate CRC
            let stored_crc = header.crc32;
            let computed_crc = Self::compute_header_crc(header);
            if stored_crc != computed_crc {
                warn!(
                    stored = stored_crc,
                    computed = computed_crc,
                    "VDir header CRC mismatch - possible corruption"
                );
                // Continue anyway, but log warning
            }
        }

        Ok(Self { mmap, capacity })
    }

    /// Compute CRC32 of header fields (excluding crc32 field itself)
    fn compute_header_crc(header: &VDirHeader) -> u32 {
        // CRC32 of first 28 bytes (magic + version + generation + entry_count + table_capacity + table_offset)
        let bytes = unsafe {
            std::slice::from_raw_parts(
                header as *const VDirHeader as *const u8,
                28, // Bytes before crc32 field
            )
        };
        crc32fast::hash(bytes)
    }

    /// Get header reference
    fn header(&self) -> &VDirHeader {
        unsafe { &*(self.mmap.as_ptr() as *const VDirHeader) }
    }

    /// Get mutable header reference
    fn header_mut(&mut self) -> &mut VDirHeader {
        unsafe { &mut *(self.mmap.as_mut_ptr() as *mut VDirHeader) }
    }

    /// Get entry table slice
    fn entries(&self) -> &[VDirEntry] {
        let offset = self.header().table_offset as usize;
        unsafe {
            std::slice::from_raw_parts(
                self.mmap.as_ptr().add(offset) as *const VDirEntry,
                self.capacity,
            )
        }
    }

    /// Get mutable entry table slice
    fn entries_mut(&mut self) -> &mut [VDirEntry] {
        let offset = self.header().table_offset as usize;
        unsafe {
            std::slice::from_raw_parts_mut(
                self.mmap.as_mut_ptr().add(offset) as *mut VDirEntry,
                self.capacity,
            )
        }
    }

    /// Seqlock writer: begin a write transaction.
    /// Stores current_gen + 1 (odd) with Release ordering to signal "write in progress".
    /// Readers seeing an odd generation will spin-wait.
    pub fn begin_write(&mut self) {
        let gen_ptr = &self.header().generation as *const u64;
        let atomic = unsafe { &*(gen_ptr as *const AtomicU64) };
        let current = atomic.load(Ordering::Relaxed);
        debug_assert!(
            current & 1 == 0,
            "begin_write called while already writing (gen={})",
            current
        );
        atomic.store(current + 1, Ordering::Release);
    }

    /// Seqlock writer: end a write transaction.
    /// Stores current_gen + 1 (even) with Release ordering to signal "data stable".
    /// Also recomputes header CRC.
    pub fn end_write(&mut self) {
        // Recompute CRC before bumping to even (readers validate CRC after gen check)
        self.header_mut().crc32 = Self::compute_header_crc(self.header());
        let gen_ptr = &self.header().generation as *const u64;
        let atomic = unsafe { &*(gen_ptr as *const AtomicU64) };
        let current = atomic.load(Ordering::Relaxed);
        debug_assert!(
            current & 1 == 1,
            "end_write called without begin_write (gen={})",
            current
        );
        atomic.store(current + 1, Ordering::Release);
    }

    /// Find slot for path hash (linear probing)
    fn find_slot(&self, path_hash: u64) -> Option<usize> {
        let start = (path_hash as usize) % self.capacity;
        for i in 0..self.capacity {
            let slot = (start + i) % self.capacity;
            let entry = &self.entries()[slot];
            if entry.is_empty() || entry.path_hash == path_hash {
                return Some(slot);
            }
        }
        None
    }

    /// Lookup entry by path hash
    pub fn lookup(&self, path_hash: u64) -> Option<&VDirEntry> {
        let start = (path_hash as usize) % self.capacity;
        for i in 0..self.capacity {
            let slot = (start + i) % self.capacity;
            let entry = &self.entries()[slot];
            if entry.is_empty() {
                return None;
            }
            if entry.path_hash == path_hash {
                return Some(entry);
            }
        }
        None
    }

    /// Insert or update entry
    pub fn upsert(&mut self, entry: VDirEntry) -> Result<()> {
        let slot = self.find_slot(entry.path_hash).context("VDir full")?;

        let existing = &self.entries()[slot];
        let is_new = existing.is_empty();

        self.begin_write();
        self.entries_mut()[slot] = entry;

        if is_new {
            self.header_mut().entry_count += 1;
        }

        self.end_write();
        Ok(())
    }

    /// Mark entry as dirty
    pub fn mark_dirty(&mut self, path_hash: u64, dirty: bool) -> bool {
        if let Some(slot) = self.find_slot(path_hash) {
            let entries = self.entries_mut();
            let entry = &mut entries[slot];
            if !entry.is_empty() && entry.path_hash == path_hash {
                self.begin_write();
                // Re-acquire mutable entry after begin_write borrows self
                let entries = self.entries_mut();
                let entry = &mut entries[slot];
                if dirty {
                    entry.flags |= FLAG_DIRTY;
                } else {
                    entry.flags &= !FLAG_DIRTY;
                }
                self.end_write();
                return true;
            }
        }
        false
    }

    /// Flush mmap to disk
    pub fn flush(&self) -> Result<()> {
        self.mmap.flush()?;
        Ok(())
    }
}

/// FNV-1a hash for paths
#[inline]
pub fn fnv1a_hash(s: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use tempfile::tempdir;

    // ==================== Basic Functionality ====================

    #[test]
    fn test_vdir_create_and_lookup() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        // Insert entry
        let entry = VDirEntry {
            path_hash: fnv1a_hash("src/main.rs"),
            cas_hash: [1; 32],
            size: 1024,
            mtime_sec: 1234567890,
            mtime_nsec: 0,
            mode: 0o644,
            flags: 0,
            _pad: [0; 3],
        };
        vdir.upsert(entry).unwrap();

        // Lookup
        let found = vdir.lookup(fnv1a_hash("src/main.rs"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().size, 1024);
    }

    #[test]
    fn test_dirty_bit() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        let entry = VDirEntry {
            path_hash: fnv1a_hash("main.o"),
            ..Default::default()
        };
        vdir.upsert(entry).unwrap();

        // Mark dirty
        assert!(vdir.mark_dirty(fnv1a_hash("main.o"), true));
        assert!(vdir.lookup(fnv1a_hash("main.o")).unwrap().is_dirty());

        // Clear dirty
        assert!(vdir.mark_dirty(fnv1a_hash("main.o"), false));
        assert!(!vdir.lookup(fnv1a_hash("main.o")).unwrap().is_dirty());
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_lookup_nonexistent_path() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let vdir = VDir::create_or_open(&path).unwrap();
        assert!(vdir.lookup(fnv1a_hash("nonexistent.txt")).is_none());
    }

    #[test]
    fn test_mark_dirty_nonexistent_returns_false() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();
        assert!(!vdir.mark_dirty(fnv1a_hash("nonexistent.txt"), true));
    }

    #[test]
    fn test_upsert_updates_existing_entry() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        // Insert
        let entry1 = VDirEntry {
            path_hash: fnv1a_hash("file.txt"),
            size: 100,
            ..Default::default()
        };
        vdir.upsert(entry1).unwrap();
        assert_eq!(vdir.lookup(fnv1a_hash("file.txt")).unwrap().size, 100);

        // Update with new size
        let entry2 = VDirEntry {
            path_hash: fnv1a_hash("file.txt"),
            size: 200,
            ..Default::default()
        };
        vdir.upsert(entry2).unwrap();
        assert_eq!(vdir.lookup(fnv1a_hash("file.txt")).unwrap().size, 200);
    }

    #[test]
    fn test_entry_count_increments() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();
        assert_eq!(vdir.header().entry_count, 0);

        vdir.upsert(VDirEntry {
            path_hash: fnv1a_hash("a.txt"),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(vdir.header().entry_count, 1);

        vdir.upsert(VDirEntry {
            path_hash: fnv1a_hash("b.txt"),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(vdir.header().entry_count, 2);

        // Update existing - should NOT increment
        vdir.upsert(VDirEntry {
            path_hash: fnv1a_hash("a.txt"),
            size: 999,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(vdir.header().entry_count, 2);
    }

    // ==================== Generation Counter ====================

    #[test]
    fn test_generation_increments_on_upsert() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();
        let gen_before = vdir.header().generation;

        vdir.upsert(VDirEntry {
            path_hash: fnv1a_hash("file.txt"),
            ..Default::default()
        })
        .unwrap();

        assert_eq!(vdir.header().generation, gen_before + 2);
    }

    #[test]
    fn test_generation_increments_on_dirty_mark() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        vdir.upsert(VDirEntry {
            path_hash: fnv1a_hash("file.txt"),
            ..Default::default()
        })
        .unwrap();

        let gen_before = vdir.header().generation;
        vdir.mark_dirty(fnv1a_hash("file.txt"), true);
        assert_eq!(vdir.header().generation, gen_before + 2);
    }

    // ==================== Persistence ====================

    #[test]
    fn test_vdir_persists_after_reopen() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        // Create and insert
        {
            let mut vdir = VDir::create_or_open(&path).unwrap();
            vdir.upsert(VDirEntry {
                path_hash: fnv1a_hash("persistent.txt"),
                size: 42,
                cas_hash: [7; 32],
                ..Default::default()
            })
            .unwrap();
            vdir.flush().unwrap();
        }

        // Reopen and verify
        {
            let vdir = VDir::create_or_open(&path).unwrap();
            let entry = vdir.lookup(fnv1a_hash("persistent.txt"));
            assert!(entry.is_some());
            assert_eq!(entry.unwrap().size, 42);
            assert_eq!(entry.unwrap().cas_hash, [7; 32]);
        }
    }

    #[test]
    fn test_vdir_header_valid_after_reopen() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        // Create with entries
        {
            let mut vdir = VDir::create_or_open(&path).unwrap();
            for i in 0..10 {
                vdir.upsert(VDirEntry {
                    path_hash: fnv1a_hash(&format!("file_{}.txt", i)),
                    ..Default::default()
                })
                .unwrap();
            }
            vdir.flush().unwrap();
        }

        // Reopen and verify header
        {
            let vdir = VDir::create_or_open(&path).unwrap();
            let header = vdir.header();
            assert_eq!(header.magic, VDIR_MAGIC);
            assert_eq!(header.version, VDIR_VERSION);
            assert_eq!(header.entry_count, 10);
            assert!(header.generation >= 20);
        }
    }

    // ==================== Flag Operations ====================

    #[test]
    fn test_all_flags() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        // Test DIR flag
        let dir_entry = VDirEntry {
            path_hash: fnv1a_hash("mydir/"),
            flags: FLAG_DIR,
            ..Default::default()
        };
        vdir.upsert(dir_entry).unwrap();
        assert!(vdir.lookup(fnv1a_hash("mydir/")).unwrap().is_dir());

        // Test SYMLINK flag
        let symlink_entry = VDirEntry {
            path_hash: fnv1a_hash("link"),
            flags: FLAG_SYMLINK,
            ..Default::default()
        };
        vdir.upsert(symlink_entry).unwrap();
        let e = vdir.lookup(fnv1a_hash("link")).unwrap();
        assert_eq!(e.flags & FLAG_SYMLINK, FLAG_SYMLINK);
    }

    // ==================== Stress Tests ====================

    #[test]
    fn test_insert_many_entries() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        // Insert 1000 entries
        for i in 0..1000 {
            let entry = VDirEntry {
                path_hash: fnv1a_hash(&format!("path/to/file_{}.rs", i)),
                size: i as u64,
                ..Default::default()
            };
            vdir.upsert(entry).unwrap();
        }

        assert_eq!(vdir.header().entry_count, 1000);

        // Verify random lookups
        assert_eq!(
            vdir.lookup(fnv1a_hash("path/to/file_0.rs")).unwrap().size,
            0
        );
        assert_eq!(
            vdir.lookup(fnv1a_hash("path/to/file_500.rs")).unwrap().size,
            500
        );
        assert_eq!(
            vdir.lookup(fnv1a_hash("path/to/file_999.rs")).unwrap().size,
            999
        );
    }

    #[test]
    fn test_concurrent_reads() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        // Setup: insert entries
        {
            let mut vdir = VDir::create_or_open(&path).unwrap();
            for i in 0..100 {
                vdir.upsert(VDirEntry {
                    path_hash: fnv1a_hash(&format!("file_{}", i)),
                    size: i as u64,
                    ..Default::default()
                })
                .unwrap();
            }
            vdir.flush().unwrap();
        }

        // Concurrent reads from multiple threads
        let path = Arc::new(path);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let p = Arc::clone(&path);
                thread::spawn(move || {
                    let vdir = VDir::create_or_open(&p).unwrap();
                    for i in 0..100 {
                        let entry = vdir.lookup(fnv1a_hash(&format!("file_{}", i)));
                        assert!(entry.is_some());
                        assert_eq!(entry.unwrap().size, i as u64);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    // ==================== Hash Function Tests ====================

    #[test]
    fn test_fnv1a_deterministic() {
        let hash1 = fnv1a_hash("src/main.rs");
        let hash2 = fnv1a_hash("src/main.rs");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_fnv1a_different_paths_different_hashes() {
        let hash1 = fnv1a_hash("a.txt");
        let hash2 = fnv1a_hash("b.txt");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_fnv1a_empty_string() {
        let hash = fnv1a_hash("");
        // FNV offset basis
        assert_eq!(hash, 0xcbf29ce484222325);
    }

    #[test]
    fn test_fnv1a_long_path() {
        let long_path = "a/".repeat(500) + "file.txt";
        let hash = fnv1a_hash(&long_path);
        assert_ne!(hash, 0);
    }

    // ==================== Seqlock Protocol ====================

    #[test]
    fn test_seqlock_begin_end_bracket() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();
        assert_eq!(vdir.header().generation, 0); // starts even

        vdir.begin_write();
        assert_eq!(vdir.header().generation & 1, 1); // odd = writing

        vdir.end_write();
        assert_eq!(vdir.header().generation & 1, 0); // even = stable
        assert_eq!(vdir.header().generation, 2);
    }

    #[test]
    fn test_seqlock_generation_always_even_after_write() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();

        for i in 0..50 {
            vdir.upsert(VDirEntry {
                path_hash: fnv1a_hash(&format!("file_{}.rs", i)),
                size: i as u64,
                ..Default::default()
            })
            .unwrap();
            // After every upsert, generation must be even
            assert_eq!(
                vdir.header().generation & 1,
                0,
                "generation odd after upsert #{}",
                i
            );
        }

        // mark_dirty also leaves generation even
        for i in 0..10 {
            vdir.mark_dirty(fnv1a_hash(&format!("file_{}.rs", i)), true);
            assert_eq!(
                vdir.header().generation & 1,
                0,
                "generation odd after mark_dirty #{}",
                i
            );
        }
    }

    #[test]
    fn test_seqlock_concurrent_reader_writer() {
        use std::sync::atomic::AtomicBool;

        let temp = tempdir().unwrap();
        let path = temp.path().join("test.vdir");

        // Setup: create VDir with initial entries
        {
            let mut vdir = VDir::create_or_open(&path).unwrap();
            for i in 0..100 {
                vdir.upsert(VDirEntry {
                    path_hash: fnv1a_hash(&format!("file_{}", i)),
                    size: i as u64,
                    ..Default::default()
                })
                .unwrap();
            }
            vdir.flush().unwrap();
        }

        let done = Arc::new(AtomicBool::new(false));
        let path_arc = Arc::new(path);

        // Writer thread: continuously upsert entries
        let done_w = done.clone();
        let path_w = path_arc.clone();
        let writer = thread::spawn(move || {
            let mut vdir = VDir::create_or_open(&path_w).unwrap();
            let mut round = 0u64;
            while !done_w.load(Ordering::Relaxed) {
                let idx = (round % 100) as usize;
                vdir.upsert(VDirEntry {
                    path_hash: fnv1a_hash(&format!("file_{}", idx)),
                    size: round,
                    ..Default::default()
                })
                .unwrap();
                round += 1;
            }
            round
        });

        // Reader threads: observe generation values
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let d = done.clone();
                let p = path_arc.clone();
                thread::spawn(move || {
                    let vdir = VDir::create_or_open(&p).unwrap();
                    let mut reads = 0u64;
                    let mut retries = 0u64;
                    while !d.load(Ordering::Relaxed) {
                        let gen = vdir.header().generation;
                        if gen & 1 == 1 {
                            // Writer is active — expected under concurrency
                            retries += 1;
                            core::hint::spin_loop();
                            continue;
                        }
                        // Read a sample entry while gen is even
                        let _entry = vdir.lookup(fnv1a_hash("file_0"));
                        reads += 1;
                    }
                    (reads, retries)
                })
            })
            .collect();

        // Let it run for ~200ms
        thread::sleep(std::time::Duration::from_millis(200));
        done.store(true, Ordering::Relaxed);

        let write_rounds = writer.join().unwrap();
        let mut total_reads = 0u64;
        for r in readers {
            let (reads, _retries) = r.join().unwrap();
            total_reads += reads;
        }

        // Basic sanity: writer did work, readers succeeded
        assert!(write_rounds > 0, "writer did no work");
        assert!(total_reads > 0, "readers did no reads");
    }
}
