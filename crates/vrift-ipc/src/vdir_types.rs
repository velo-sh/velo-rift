//! VDir shared types — SSOT for both vDird (writer) and InceptionLayer (reader)
//!
//! These types define the on-disk/mmap layout of the VDir hash table.
//! Any field changes here MUST maintain `#[repr(C)]` ABI stability.

/// VDir magic number: "VRFT" in little-endian
pub const VDIR_MAGIC: u32 = 0x56524654;

/// VDir format version. Bump on incompatible changes.
pub const VDIR_VERSION: u32 = 2; // v2: Added CRC32 checksum

/// Default hash table capacity (slots)
pub const VDIR_DEFAULT_CAPACITY: usize = 65536;

/// Compile-time entry size (for offset calculations)
pub const VDIR_ENTRY_SIZE: usize = std::mem::size_of::<VDirEntry>();

/// Compile-time header size
pub const VDIR_HEADER_SIZE: usize = std::mem::size_of::<VDirHeader>();

// ---------------------------------------------------------------------------
// Flag definitions
// ---------------------------------------------------------------------------

/// Entry has pending writes in staging (not yet committed to CAS)
pub const FLAG_DIRTY: u16 = 0x0001;
/// Entry has been logically deleted
pub const FLAG_DELETED: u16 = 0x0002;
/// Entry is a symlink
pub const FLAG_SYMLINK: u16 = 0x0004;
/// Entry is a directory
pub const FLAG_DIR: u16 = 0x0008;

// ---------------------------------------------------------------------------
// VDirHeader — occupies first 64 bytes of the mmap file
// ---------------------------------------------------------------------------

/// VDir header in shared memory.
///
/// Layout (64 bytes total):
/// ```text
/// offset  field             size
/// ------  ---------------   ----
///  0      magic             4    (0x56524654)
///  4      version           4
///  8      generation        8    (seqlock counter, must be AtomicU64-aligned)
/// 16      entry_count       4
/// 20      table_capacity    4
/// 24      table_offset      4
/// 28      crc32             4
/// 32      _pad             32
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VDirHeader {
    pub magic: u32,
    pub version: u32,
    pub generation: u64, // Atomic counter for seqlock synchronization
    pub entry_count: u32,
    pub table_capacity: u32,
    pub table_offset: u32,
    pub crc32: u32,     // CRC32 checksum of header (fields before crc32)
    pub _pad: [u8; 32], // Pad to 64 bytes
}

// Compile-time assertion: VDirHeader must be exactly 64 bytes
const _: () = assert!(std::mem::size_of::<VDirHeader>() == 64);

// ---------------------------------------------------------------------------
// VDirEntry — 72 bytes per slot in the hash table
// ---------------------------------------------------------------------------

/// Single VDir entry in the hash table (open addressing, linear probing).
///
/// Layout (72 bytes total):
/// ```text
/// offset  field         size
/// ------  -----------   ----
///  0      path_hash      8   (FNV-1a, 0 = empty slot)
///  8      cas_hash      32   (BLAKE3 content hash)
/// 40      size           8
/// 48      mtime_sec      8
/// 56      mtime_nsec     4
/// 60      mode           4
/// 64      flags          2
/// 66      _pad           6
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VDirEntry {
    pub path_hash: u64,     // FNV-1a hash of path (0 = empty slot)
    pub cas_hash: [u8; 32], // BLAKE3 content hash
    pub size: u64,
    pub mtime_sec: i64,
    pub mtime_nsec: u32,
    pub mode: u32,
    pub flags: u16, // FLAG_DIRTY | FLAG_DELETED | FLAG_SYMLINK | FLAG_DIR
    pub _pad: [u16; 3],
}

// Compile-time assertion: VDirEntry must be exactly 72 bytes
const _: () = assert!(std::mem::size_of::<VDirEntry>() == 72);

impl VDirEntry {
    /// True if slot is empty (never written)
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.path_hash == 0
    }

    /// True if entry has pending writes
    #[inline]
    pub fn is_dirty(&self) -> bool {
        (self.flags & FLAG_DIRTY) != 0
    }

    /// True if entry is a directory
    #[inline]
    pub fn is_dir(&self) -> bool {
        (self.flags & FLAG_DIR) != 0
    }

    /// True if entry has been logically deleted
    #[inline]
    pub fn is_deleted(&self) -> bool {
        (self.flags & FLAG_DELETED) != 0
    }

    /// True if entry is a symlink
    #[inline]
    pub fn is_symlink(&self) -> bool {
        (self.flags & FLAG_SYMLINK) != 0
    }
}
