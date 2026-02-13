//! VDir mmap management

use anyhow::{Context, Result};
use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, info, warn};

// Re-export shared VDir types from vrift-ipc (SSOT)
pub use vrift_ipc::vdir_types::*;

/// VDir manager
pub struct VDir {
    mmap: MmapMut,
    capacity: usize,
    path: std::path::PathBuf,
}

impl VDir {
    /// Create or open existing VDir mmap file
    pub fn create_or_open(path: &Path) -> Result<Self> {
        let capacity = VDIR_DEFAULT_CAPACITY;
        let table_size = capacity * VDIR_ENTRY_SIZE;
        let string_pool_offset = VDIR_HEADER_SIZE + table_size;
        let bloom_filter_offset = string_pool_offset + VDIR_STRING_POOL_CAPACITY;
        let bloom_filter_size = 64 * 1024; // 64KB Bloom Filter
        let file_size = bloom_filter_offset + bloom_filter_size;

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
        } else if (metadata.len() as usize) < file_size {
            // v2 → v3 upgrade: expand file to include string pool
            file.set_len(file_size as u64)?;
            info!(
                path = %path.display(),
                old_size = metadata.len(),
                new_size = file_size,
                "Expanded VDir file for v3 string pool"
            );
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
                string_pool_offset: string_pool_offset as u32,
                string_pool_size: 0,
                string_pool_capacity: VDIR_STRING_POOL_CAPACITY as u32,
                bloom_offset: bloom_filter_offset as u32,
                bloom_size: bloom_filter_size as u32,
                _pad: [0; 12],
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

            // Recovery: If generation is odd, previous writer crashed mid-write.
            // Reset to even (generation + 1) so seqlock readers don't spin forever.
            if header.generation & 1 != 0 {
                let stale_gen = header.generation;
                let recovered_gen = stale_gen + 1; // next even number
                warn!(
                    stale_gen = stale_gen,
                    recovered_gen = recovered_gen,
                    "VDir generation stuck at odd value (previous writer crashed). Recovering."
                );
                let gen_ptr = &header.generation as *const u64;
                let atomic = unsafe { &*(gen_ptr as *const AtomicU64) };
                atomic.store(recovered_gen, Ordering::Release);
                // Recompute CRC with updated generation
                header.crc32 = Self::compute_header_crc(header);
                mmap.flush()?;
            }
        }

        Ok(Self {
            mmap,
            capacity,
            path: path.to_path_buf(),
        })
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

    /// Open an existing VDir in read-only mode (for observability)
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .context("Failed to open VDir file in read-only mode")?;

        let mmap_ro = unsafe { memmap2::Mmap::map(&file)? };
        let header = unsafe { &*(mmap_ro.as_ptr() as *const VDirHeader) };

        if header.magic != VDIR_MAGIC {
            anyhow::bail!("Invalid VDir magic: {:x}", header.magic);
        }

        let capacity = header.table_capacity as usize;

        // For read-only mode, we still store it in the MmapMut field via transition.
        // We MUST NOT call mutable methods if opened this way.
        let mmap = unsafe { std::mem::transmute::<memmap2::Mmap, MmapMut>(mmap_ro) };

        Ok(Self {
            mmap,
            capacity,
            path: path.to_path_buf(),
        })
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
        // Dynamic Resize: Check if resulting load factor would exceed 75%
        let current_count = self.header().entry_count as usize;
        let existing_entry = self.lookup(entry.path_hash);
        let is_new = existing_entry.is_none();

        if is_new && (current_count + 1) as f64 / self.capacity as f64 > 0.75 {
            self.resize(self.capacity * 2)?;
        }

        let slot = self.find_slot(entry.path_hash).context("VDir full")?;

        let existing = &self.entries()[slot];
        let is_new = existing.is_empty();

        self.begin_write();
        self.update_bloom_filter(entry.path_hash);
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

    /// Calculate VDir statistics for observability
    pub fn get_stats(&self) -> VDirStats {
        let entries = self.entries();
        let capacity = self.capacity;
        let mut occupied = 0;
        let mut max_chain = 0;
        let mut total_chain = 0;

        for (i, entry) in entries.iter().enumerate().take(capacity) {
            if !entry.is_empty() {
                occupied += 1;

                // Calculate collision chain length for this entry
                let ideal_slot = (entry.path_hash as usize) % capacity;
                let actual_slot = i;
                let chain_len = if actual_slot >= ideal_slot {
                    actual_slot - ideal_slot + 1
                } else {
                    (capacity - ideal_slot) + actual_slot + 1
                };

                max_chain = max_chain.max(chain_len);
                total_chain += chain_len;
            }
        }

        let load_factor = if capacity > 0 {
            occupied as f64 / capacity as f64
        } else {
            0.0
        };

        let avg_chain = if occupied > 0 {
            total_chain as f64 / occupied as f64
        } else {
            0.0
        };

        VDirStats {
            capacity,
            entry_count: occupied,
            load_factor,
            max_collision_chain: max_chain,
            avg_collision_chain: avg_chain,
            generation: self.header().generation,
        }
    }

    /// Resize VDir to a new capacity.
    /// Rehashes all existing entries into a larger table.
    /// String pool is preserved and compacted during resize.
    pub fn resize(&mut self, new_capacity: usize) -> Result<()> {
        info!(
            "vdir: Resizing from {} to {} entries...",
            self.capacity, new_capacity
        );

        // 1. Snapshot existing entries and their paths
        let entries_snapshot: Vec<(VDirEntry, Option<String>)> = self
            .entries()
            .iter()
            .filter(|e| !e.is_empty())
            .map(|e| {
                let path = self.get_path(e).map(|s| s.to_string());
                (*e, path)
            })
            .collect();

        // 2. Resize file and remap (include string pool)
        let file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        let table_size = new_capacity * VDIR_ENTRY_SIZE;
        let new_pool_offset = VDIR_HEADER_SIZE + table_size;
        let new_size = new_pool_offset + VDIR_STRING_POOL_CAPACITY;
        file.set_len(new_size as u64)?;

        // Re-map MmapMut
        self.mmap = unsafe { MmapMut::map_mut(&file)? };
        self.capacity = new_capacity;

        // 3. Update header
        self.begin_write();
        let header = self.header_mut();
        header.table_capacity = new_capacity as u32;
        header.entry_count = 0;
        header.string_pool_offset = new_pool_offset as u32;
        header.string_pool_size = 0; // Reset — will be repopulated
        header.string_pool_capacity = VDIR_STRING_POOL_CAPACITY as u32;

        // 4. Clear table (zero out)
        let entries_ptr = unsafe { self.mmap.as_mut_ptr().add(VDIR_HEADER_SIZE) };
        unsafe {
            std::ptr::write_bytes(entries_ptr, 0, table_size);
        }

        // 5. Clear string pool
        let pool_ptr = unsafe { self.mmap.as_mut_ptr().add(new_pool_offset) };
        unsafe {
            std::ptr::write_bytes(pool_ptr, 0, VDIR_STRING_POOL_CAPACITY);
        }

        // 6. Re-insert with paths (rehash + compact pool)
        for (mut entry, path) in entries_snapshot {
            // Re-allocate path in new pool if available
            if let Some(ref p) = path {
                if let Some((offset, len)) = self.string_pool_alloc(p.as_bytes()) {
                    entry.path_offset = offset;
                    entry.path_len = len;
                } else {
                    entry.path_offset = 0;
                    entry.path_len = 0;
                }
            } else {
                entry.path_offset = 0;
                entry.path_len = 0;
            }

            let slot = self
                .find_slot(entry.path_hash)
                .context("VDir full after resize")?;
            self.entries_mut()[slot] = entry;
            self.header_mut().entry_count += 1;
        }

        self.end_write();
        self.flush()?;

        info!("vdir: Resize complete.");
        Ok(())
    }

    /// Helper to get parent path for V4 directory index
    fn get_parent_path(path: &str) -> Option<&str> {
        if path == "/" || path.is_empty() {
            return None;
        }
        let trimmed = path.trim_end_matches('/');
        if let Some(idx) = trimmed.rfind('/') {
            if idx == 0 {
                Some("/")
            } else {
                Some(&trimmed[..idx])
            }
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // String Pool — bump allocator for path strings
    // -----------------------------------------------------------------------

    /// Allocate space in the string pool and write data.
    /// Returns (offset_within_pool, len) or None if pool is full.
    fn string_pool_alloc(&mut self, data: &[u8]) -> Option<(u32, u16)> {
        if data.is_empty() || data.len() > u16::MAX as usize {
            return None;
        }

        let header = self.header();
        let pool_offset = header.string_pool_offset as usize;
        let pool_used = header.string_pool_size as usize;
        let pool_cap = header.string_pool_capacity as usize;

        if pool_used + data.len() > pool_cap {
            warn!(
                used = pool_used,
                needed = data.len(),
                cap = pool_cap,
                "String pool full"
            );
            return None;
        }

        let write_offset = pool_offset + pool_used;
        let data_len = data.len() as u16;
        let pool_relative_offset = pool_used as u32;

        // Write data into mmap
        self.mmap[write_offset..write_offset + data.len()].copy_from_slice(data);

        // Update pool size in header
        self.header_mut().string_pool_size = (pool_used + data.len()) as u32;

        Some((pool_relative_offset, data_len))
    }

    /// Read a path string from the string pool for a given entry.
    pub fn get_path(&self, entry: &VDirEntry) -> Option<&str> {
        if entry.path_len == 0 {
            return None;
        }

        let header = self.header();
        let pool_offset = header.string_pool_offset as usize;
        let start = pool_offset + entry.path_offset as usize;
        let end = start + entry.path_len as usize;

        if end > self.mmap.len() {
            return None;
        }

        std::str::from_utf8(&self.mmap[start..end]).ok()
    }

    /// Insert or update entry with path string.
    /// The path is stored in the string pool for readdir enumeration.
    pub fn upsert_with_path(&mut self, mut entry: VDirEntry, path: &str) -> Result<()> {
        // Dynamic Resize: Check if resulting load factor would exceed 75%
        let current_count = self.header().entry_count as usize;
        let existing = self.lookup(entry.path_hash);
        let is_new = existing.is_none();

        if is_new && (current_count + 1) as f64 / self.capacity as f64 > 0.75 {
            self.resize(self.capacity * 2)?;
        }

        // Allocate path in string pool (skip if already stored with same content)
        let should_alloc = if let Some(existing) = self.lookup(entry.path_hash) {
            // Entry exists — check if path changed or was never stored
            existing.path_len == 0
        } else {
            true
        };

        if should_alloc {
            if let Some((offset, len)) = self.string_pool_alloc(path.as_bytes()) {
                entry.path_offset = offset;
                entry.path_len = len;
            }
            // If pool full: entry works without path (no readdir, but open/stat still work)
        } else if let Some(existing) = self.lookup(entry.path_hash) {
            // Preserve existing path offset/len
            entry.path_offset = existing.path_offset;
            entry.path_len = existing.path_len;
        }

        let slot = self.find_slot(entry.path_hash).context("VDir full")?;
        let is_new = self.entries()[slot].is_empty();

        if is_new {
            entry.first_child_idx = u32::MAX;
            entry.next_sibling_idx = u32::MAX;

            if let Some(parent_path) = Self::get_parent_path(path) {
                let parent_hash = fnv1a_hash(parent_path);
                entry.parent_hash = parent_hash;

                // Note: We'll link to parent under the write lock below
            }
        } else {
            // Preserve existing V4 links on update
            let existing = &self.entries()[slot];
            entry.parent_hash = existing.parent_hash;
            entry.first_child_idx = existing.first_child_idx;
            entry.next_sibling_idx = existing.next_sibling_idx;
        }

        self.begin_write();

        // V4: Update Bloom Filter for fast O(1) miss detection
        self.update_bloom_filter(entry.path_hash);

        // V4: If new entry, link into parent's children list
        if is_new && entry.parent_hash != 0 {
            if let Some(parent_slot) = self.find_slot(entry.parent_hash) {
                let parent_exists = !self.entries()[parent_slot].is_empty();
                if parent_exists {
                    entry.next_sibling_idx = self.entries()[parent_slot].first_child_idx;
                    self.entries_mut()[parent_slot].first_child_idx = slot as u32;
                }
            }
        }

        self.entries_mut()[slot] = entry;

        if is_new {
            self.header_mut().entry_count += 1;
        }

        self.end_write();
        Ok(())
    }

    /// List all entries whose path starts with the given directory prefix.
    /// Returns (relative_name, entry) pairs for immediate children only.
    ///
    /// Example: list_directory("target/debug") returns entries like
    ///   ("deps", entry), (".fingerprint", entry), etc.
    pub fn list_directory(&self, dir_prefix: &str) -> Vec<(String, VDirEntry)> {
        let mut results = Vec::new();
        let prefix = if dir_prefix.ends_with('/') {
            dir_prefix.to_string()
        } else {
            format!("{}/", dir_prefix)
        };

        // VDIR V4: Try O(1) readdir via linked list
        if self.header().version >= 4 {
            let normalized_dir = if dir_prefix == "/" {
                "/"
            } else {
                dir_prefix.trim_end_matches('/')
            };
            let dir_hash = fnv1a_hash(normalized_dir);

            if let Some(dir_slot) = self.find_slot(dir_hash) {
                let dir_entry = &self.entries()[dir_slot];
                if !dir_entry.is_empty() && dir_entry.is_dir() {
                    let mut current_idx = dir_entry.first_child_idx;
                    while current_idx != u32::MAX && (current_idx as usize) < self.capacity {
                        let child_entry = &self.entries()[current_idx as usize];
                        if !child_entry.is_empty() && !child_entry.is_deleted() {
                            if let Some(path) = self.get_path(child_entry) {
                                if let Some(rest) = path.strip_prefix(&prefix) {
                                    let child_name = if let Some(slash_pos) = rest.find('/') {
                                        &rest[..slash_pos]
                                    } else {
                                        rest
                                    };

                                    if !child_name.is_empty()
                                        && !results.iter().any(|(n, _)| n == child_name)
                                    {
                                        results.push((child_name.to_string(), *child_entry));
                                    }
                                }
                            }
                        }
                        current_idx = child_entry.next_sibling_idx;
                    }
                }
            }
        }

        // FALLBACK: Full scan if V4 index is missing or empty
        if results.is_empty() {
            let entries = self.entries();
            for entry in entries.iter().take(self.capacity) {
                if entry.is_empty() || entry.is_deleted() {
                    continue;
                }

                if let Some(path) = self.get_path(entry) {
                    if let Some(rest) = path.strip_prefix(&prefix) {
                        let child_name = if let Some(slash_pos) = rest.find('/') {
                            &rest[..slash_pos]
                        } else {
                            rest
                        };

                        if !child_name.is_empty() {
                            let already = results.iter().any(|(n, _)| n == child_name);
                            if !already {
                                let is_subdir = rest.contains('/');
                                let mut child_entry = *entry;
                                if is_subdir {
                                    child_entry.flags |= FLAG_DIR;
                                    child_entry.size = 0;
                                }
                                results.push((child_name.to_string(), child_entry));
                            }
                        }
                    }
                }
            }
        }

        results
    }

    fn update_bloom_filter(&mut self, path_hash: u64) {
        let (offset, size) = {
            let h = self.header();
            (h.bloom_offset as usize, h.bloom_size as usize)
        };
        if size == 0 || offset == 0 {
            return;
        }

        let bits = (size * 8) as u64;
        let h1 = path_hash;
        let h2 = path_hash.rotate_right(21);
        let h3 = path_hash.rotate_right(42);

        let idx1 = (h1 % bits) as usize;
        let idx2 = (h2 % bits) as usize;
        let idx3 = (h3 % bits) as usize;

        self.mmap[offset + (idx1 / 8)] |= 1 << (idx1 % 8);
        self.mmap[offset + (idx2 / 8)] |= 1 << (idx2 % 8);
        self.mmap[offset + (idx3 / 8)] |= 1 << (idx3 % 8);
    }
}

/// VDir statistics for observability
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct VDirStats {
    pub capacity: usize,
    pub entry_count: usize,
    pub load_factor: f64,
    pub max_collision_chain: usize,
    pub avg_collision_chain: f64,
    pub generation: u64,
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
            path_offset: 0,
            path_len: 0,
            parent_hash: 0,
            first_child_idx: u32::MAX,
            next_sibling_idx: u32::MAX,
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
                    let vdir = VDir::open_readonly(&p).unwrap();
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
                    let vdir = VDir::open_readonly(&p).unwrap();
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

    // ==================== Seqlock Integration Tests ====================

    /// Test seqlock reader protocol: writer does begin_write/upsert/end_write,
    /// reader threads use raw pointer + seqlock protocol and verify no torn reads.
    #[test]
    fn test_seqlock_reader_writer_consistency() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Barrier;

        let temp = tempdir().unwrap();
        let path = temp.path().join("seqlock_rw.vdir");

        // Create VDir and seed initial entries
        let mut vdir = VDir::create_or_open(&path).unwrap();
        for i in 0u64..10 {
            vdir.upsert(VDirEntry {
                path_hash: fnv1a_hash(&format!("file_{}", i)),
                size: 0,
                mtime_sec: 0,
                mode: 0o644,
                ..Default::default()
            })
            .unwrap();
        }

        // Share raw pointer from VDir's MmapMut (same physical pages via MAP_SHARED)
        let mmap_addr = vdir.mmap.as_ptr() as usize;
        let mmap_len = vdir.mmap.len();

        let done = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(5)); // 1 writer + 4 readers

        // Spawn 4 reader threads using seqlock protocol
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let d = done.clone();
                let b = barrier.clone();
                thread::spawn(move || {
                    b.wait();
                    let mut reads = 0u64;
                    let mut retries = 0u64;

                    while !d.load(Ordering::Relaxed) {
                        let mmap_ptr = mmap_addr as *const u8;

                        // Seqlock protocol: read gen, read data, read gen again
                        let gen_ptr = unsafe { &*((mmap_ptr as usize + 8) as *const AtomicU64) };
                        let g1 = gen_ptr.load(Ordering::Acquire);
                        if g1 & 1 != 0 {
                            retries += 1;
                            core::hint::spin_loop();
                            continue;
                        }

                        let table_capacity =
                            unsafe { *((mmap_ptr as usize + 20) as *const u32) } as usize;
                        let table_offset =
                            unsafe { *((mmap_ptr as usize + 24) as *const u32) } as usize;

                        if table_capacity == 0 {
                            continue;
                        }

                        // Lookup file_0
                        let target_hash = vrift_ipc::fnv1a_hash("file_0");
                        let start = (target_hash as usize) % table_capacity;
                        let mut found: Option<(u64, i64)> = None;

                        for j in 0..table_capacity {
                            let slot = (start + j) % table_capacity;
                            let off = table_offset + slot * VDIR_ENTRY_SIZE;
                            if off + VDIR_ENTRY_SIZE > mmap_len {
                                break;
                            }
                            let e = unsafe { &*(mmap_ptr.add(off) as *const VDirEntry) };
                            if e.path_hash == 0 {
                                break;
                            }
                            if e.path_hash == target_hash {
                                found = Some((e.size, e.mtime_sec));
                                break;
                            }
                        }

                        let g2 = gen_ptr.load(Ordering::Acquire);
                        if g1 != g2 {
                            retries += 1;
                            continue;
                        }

                        // Verify consistency: size = round * 1000, mtime_sec = round
                        if let Some((size, mtime)) = found {
                            assert_eq!(
                                size % 1000,
                                0,
                                "Torn read: size={} not multiple of 1000",
                                size
                            );
                            assert_eq!(
                                mtime,
                                (size / 1000) as i64,
                                "Inconsistent: size={} mtime={}",
                                size,
                                mtime
                            );
                        }

                        reads += 1;
                    }
                    (reads, retries)
                })
            })
            .collect();

        // Writer runs in main thread (owns VDir)
        barrier.wait();
        let mut rounds = 0u64;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        while std::time::Instant::now() < deadline {
            for i in 0u64..10 {
                vdir.upsert(VDirEntry {
                    path_hash: fnv1a_hash(&format!("file_{}", i)),
                    size: rounds * 1000,
                    mtime_sec: rounds as i64,
                    mode: 0o644,
                    ..Default::default()
                })
                .unwrap();
            }
            rounds += 1;
        }
        done.store(true, Ordering::Relaxed);

        let mut total_reads = 0u64;
        for r in readers {
            let (reads, _retries) = r.join().unwrap();
            total_reads += reads;
        }
        assert!(rounds > 0, "writer did no work");
        assert!(total_reads > 0, "readers got no consistent reads");
    }

    /// Test generation recovery: simulate a crash by writing odd generation directly,
    /// then reopen and verify recovery to even.
    #[test]
    fn test_generation_recovery_after_crash() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("crash_recovery.vdir");

        // Create a valid VDir with one entry
        {
            let mut vdir = VDir::create_or_open(&path).unwrap();
            vdir.upsert(VDirEntry {
                path_hash: fnv1a_hash("orphan_file"),
                size: 42,
                ..Default::default()
            })
            .unwrap();
            vdir.flush().unwrap();

            // Simulate crash: force odd generation via direct atomic write
            let gen_ptr = &vdir.header().generation as *const u64;
            let atomic = unsafe { &*(gen_ptr as *const AtomicU64) };
            let current = atomic.load(Ordering::Relaxed);
            assert!(current & 1 == 0, "Should be even before crash sim");
            atomic.store(current + 1, Ordering::Release);
            vdir.flush().unwrap();
        }

        // Reopen — should recover generation to even
        {
            let vdir = VDir::create_or_open(&path).unwrap();
            let gen = vdir.header().generation;
            assert!(
                gen & 1 == 0,
                "Generation should be even after recovery, got {}",
                gen
            );
            let entry = vdir.lookup(fnv1a_hash("orphan_file"));
            assert!(entry.is_some(), "Entry should survive crash recovery");
            assert_eq!(entry.unwrap().size, 42);
        }
    }

    /// Test automatic resizing when load factor > 0.75
    #[test]
    fn test_vdir_resize() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("resize_test.vdir");

        // Use real VDir (starts at VDIR_DEFAULT_CAPACITY = 65536)
        let mut vdir = VDir::create_or_open(&path).unwrap();
        let initial_capacity = vdir.capacity;

        // Insert until it exceeds 75%
        // Using a loop to insert many entries
        let target = (initial_capacity as f64 * 0.76) as usize;
        info!("Inserting {} entries to trigger resize...", target);

        for i in 0..target {
            vdir.upsert(VDirEntry {
                path_hash: i as u64 + 1001,
                size: i as u64,
                ..Default::default()
            })
            .unwrap();
        }

        // Capacity should have doubled
        assert_eq!(vdir.capacity, initial_capacity * 2);
        assert_eq!(vdir.header().table_capacity as usize, initial_capacity * 2);

        // Verify we can still find the first entry
        let entry = vdir
            .lookup(1001)
            .expect("Entry 1001 not found after resize");
        assert_eq!(entry.size, 0);

        // Verify we can find the last entry
        let entry = vdir
            .lookup(target as u64 + 1001 - 1)
            .expect("Last entry not found after resize");
        assert_eq!(entry.size, target as u64 - 1);

        // Statistics should reflect growth
        let stats = vdir.get_stats();
        assert_eq!(stats.capacity, initial_capacity * 2);
        assert!(stats.load_factor < 0.5); // 0.76 / 2
    }

    /// Test resizing exactly at the threshold
    #[test]
    fn test_vdir_resize_threshold_boundary() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("threshold_test.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();
        let initial_capacity = vdir.capacity;

        let threshold = (initial_capacity as f64 * 0.75) as usize;

        // Fill up to threshold
        for i in 0..threshold {
            vdir.upsert(VDirEntry {
                path_hash: i as u64 + 1,
                ..Default::default()
            })
            .unwrap();
        }
        // Our check is > 0.75. 49152 / 65536 is exactly 0.75.
        assert_eq!(
            vdir.capacity, initial_capacity,
            "Should NOT resize at exactly 75% load"
        );

        // Add one more to exceed 75%
        vdir.upsert(VDirEntry {
            path_hash: threshold as u64 + 1,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            vdir.capacity,
            initial_capacity * 2,
            "Should resize after exceeding 75% load"
        );
    }

    /// Test multiple sequential resizes
    #[test]
    fn test_vdir_multi_resize() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("multi_resize.vdir");

        let mut vdir = VDir::create_or_open(&path).unwrap();
        let initial_capacity = vdir.capacity;

        // Trigger 1st resize (65k -> 131k)
        let target1 = (initial_capacity as f64 * 0.76) as usize;
        for i in 0..target1 {
            vdir.upsert(VDirEntry {
                path_hash: i as u64 + 1,
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(vdir.capacity, initial_capacity * 2);

        // Trigger 2nd resize (131k -> 262k)
        let target2 = (vdir.capacity as f64 * 0.76) as usize;
        for i in target1..target2 {
            vdir.upsert(VDirEntry {
                path_hash: i as u64 + 1,
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(vdir.capacity, initial_capacity * 4);

        // Verify lookups across the range
        assert!(vdir.lookup(1).is_some());
        assert!(vdir.lookup(target1 as u64).is_some());
        assert!(vdir.lookup(target2 as u64).is_some());

        // Verify statistics
        let stats = vdir.get_stats();
        assert_eq!(stats.capacity, initial_capacity * 4);
        assert_eq!(stats.entry_count, target2);
    }
}
