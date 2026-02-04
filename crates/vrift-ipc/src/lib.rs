use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum VeloRequest {
    Handshake {
        client_version: String,
    },
    Status,
    Spawn {
        command: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
    },
    CasInsert {
        hash: [u8; 32],
        size: u64,
    },
    CasGet {
        hash: [u8; 32],
    },
    Protect {
        path: String,
        immutable: bool,
        owner: Option<String>,
    },
    ManifestGet {
        path: String,
    },
    /// Manifest payload
    ManifestUpsert {
        path: String,
        entry: VnodeEntry,
    },
    /// RFC-0047: Remove a manifest entry (for unlink/rmdir)
    ManifestRemove {
        path: String,
    },
    /// RFC-0047: Rename/move a manifest entry
    ManifestRename {
        old_path: String,
        new_path: String,
    },
    /// RFC-0047: Update manifest mtime (for utimes/touch)
    ManifestUpdateMtime {
        path: String,
        mtime_ns: u64,
    },
    /// RFC-0047: Reingest a modified temp file back to CAS and Manifest (for CoW close)
    ManifestReingest {
        /// Virtual path in the VFS (where it should appear in Manifest)
        vpath: String,
        /// Actual temp file path to read and hash
        temp_path: String,
    },
    /// List directory entries for VFS synthesis
    ManifestListDir {
        path: String,
    },
    /// RFC-0049: Acquire advisory lock on logical file
    FlockAcquire {
        path: String,
        operation: i32, // e.g. LOCK_EX, LOCK_SH, LOCK_NB
    },
    /// RFC-0049: Release advisory lock on logical file
    FlockRelease {
        path: String,
    },
    /// Trigger Garbage Collection using a Bloom Filter of active hashes
    CasSweep {
        /// Bloom Filter of all active hashes in the manifest
        bloom_filter: Vec<u8>,
    },
    /// Register a workspace with the daemon
    RegisterWorkspace {
        /// The absolute path to the project root
        project_root: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[cfg(feature = "manifest")]
pub use vrift_manifest::VnodeEntry;

#[cfg(not(feature = "manifest"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VnodeEntry {
    pub content_hash: [u8; 32],
    pub size: u64,
    pub mtime: u64,
    pub mode: u32,
    pub flags: u16,
    #[serde(skip)]
    pub _pad: u16,
}

#[cfg(not(feature = "manifest"))]
impl VnodeEntry {
    pub fn is_dir(&self) -> bool {
        (self.flags & 1) != 0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum VeloResponse {
    HandshakeAck {
        server_version: String,
    },
    StatusAck {
        status: String,
    },
    SpawnAck {
        pid: u32,
    },
    CasAck,
    CasFound {
        size: u64,
    },
    CasNotFound,
    ManifestAck {
        entry: Option<VnodeEntry>,
    },
    /// Directory listing response for VFS synthesis
    ManifestListAck {
        entries: Vec<DirEntry>,
    },
    ProtectAck,
    /// Result of Garbage Collection sweep
    CasSweepAck {
        deleted_count: u32,
        reclaimed_bytes: u64,
    },
    /// RFC-0049: Acknowledgement for FlockAcquire/Release
    FlockAck,
    /// Acknowledge workspace registration
    RegisterAck {
        workspace_id: String,
    },
    Error(String),
}

pub fn default_socket_path() -> &'static str {
    "/tmp/vrift.sock"
}

#[cfg(feature = "cas")]
pub use vrift_cas::{bloom_hashes, BloomFilter, BLOOM_SIZE};

#[cfg(not(feature = "cas"))]
pub const BLOOM_SIZE: usize = 32 * 1024;

#[cfg(not(feature = "cas"))]
pub fn bloom_hashes(s: &str) -> (usize, usize) {
    let mut h1 = 0usize;
    let mut h2 = 0usize;
    for (i, b) in s.as_bytes().iter().enumerate() {
        h1 = h1.wrapping_add((*b as usize).wrapping_mul(i + 1));
        h2 = h2.wrapping_add((*b as usize).wrapping_mul(i + 31));
    }
    (h1, h2)
}

// ============================================================================
// Manifest Mmap Shared Memory (RFC-0044 Hot Stat Cache)
// ============================================================================

/// Magic number for manifest mmap file: "VMMP" (Vrift Manifest MmaP)
pub const MMAP_MAGIC: u32 = 0x504D4D56;
/// Current mmap format version
pub const MMAP_VERSION: u32 = 1;
/// Maximum entries in the hash table (power of 2 for fast modulo)
pub const MMAP_MAX_ENTRIES: usize = 65536;
/// Default mmap file path
pub const MMAP_DEFAULT_PATH: &str = "/tmp/vrift-manifest.mmap";

/// Header for the mmap'd manifest file
/// Layout: [Header][Bloom Filter][Hash Table]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ManifestMmapHeader {
    pub magic: u32,
    pub version: u32,
    pub entry_count: u32,
    pub bloom_offset: u32,       // Offset to bloom filter (BLOOM_SIZE)
    pub table_offset: u32,       // Offset to stat hash table (table_capacity * MmapStatEntry::SIZE)
    pub table_capacity: u32,     // Number of slots in stat hash table
    pub dir_index_offset: u32,   // Offset to directory index table
    pub dir_index_capacity: u32, // Capacity of directory index table
    pub children_offset: u32,    // Offset to children pool
    pub children_count: u32,     // Total children across all directories
}

impl ManifestMmapHeader {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn new(
        entry_count: u32,
        table_capacity: u32,
        dir_index_capacity: u32,
        children_count: u32,
    ) -> Self {
        let bloom_offset = Self::SIZE as u32;
        let table_offset = bloom_offset + BLOOM_SIZE as u32;
        let dir_index_offset = table_offset + (table_capacity * MmapStatEntry::SIZE as u32);
        let children_offset =
            dir_index_offset + (dir_index_capacity * MmapDirIndexEntry::SIZE as u32);

        Self {
            magic: MMAP_MAGIC,
            version: MMAP_VERSION,
            entry_count,
            bloom_offset,
            table_offset,
            table_capacity,
            dir_index_offset,
            dir_index_capacity,
            children_offset,
            children_count,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.magic == MMAP_MAGIC && self.version == MMAP_VERSION
    }
}

/// Single stat entry in the hash table
/// Uses open addressing with linear probing
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MmapStatEntry {
    pub path_hash: u64, // FNV-1a hash of path (0 = empty slot)
    pub size: u64,
    pub mtime: i64,
    pub mtime_nsec: i64,
    pub mode: u32,
    pub flags: u32, // EntryFlags: is_dir, is_symlink, etc.
}

impl MmapStatEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn is_empty(&self) -> bool {
        self.path_hash == 0
    }

    pub fn is_dir(&self) -> bool {
        (self.flags & 0x01) != 0
    }

    pub fn is_symlink(&self) -> bool {
        (self.flags & 0x02) != 0
    }
}

/// Directory index entry (parent -> children)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MmapDirIndexEntry {
    pub parent_hash: u64,    // FNV-1a hash of parent directory path
    pub children_start: u32, // Index into children pool
    pub children_count: u32, // Number of children
}

impl MmapDirIndexEntry {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// Child entry in the directory listing
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MmapDirChild {
    pub name: [u8; 128], // Name of the entry (max 127 bytes + null)
    pub stat_index: u32, // Index in the stat hash table (for stat-on-readdir)
    pub is_dir: u8,
    pub _pad: [u8; 3],
}

impl MmapDirChild {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    pub fn name_as_str(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

/// Calculate FNV-1a hash for path strings (deterministic, no alloc)
#[inline(always)]
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

/// Calculate total mmap file size for given capacities
pub fn mmap_file_size(
    table_capacity: usize,
    dir_index_capacity: usize,
    children_count: usize,
) -> usize {
    ManifestMmapHeader::SIZE
        + BLOOM_SIZE
        + (table_capacity * MmapStatEntry::SIZE)
        + (dir_index_capacity * MmapDirIndexEntry::SIZE)
        + (children_count * MmapDirChild::SIZE)
}

/// Builder for creating mmap manifest files (RFC-0044 Hot Stat Cache)
/// Used by daemon to export manifest to shared memory for O(1) shim access
#[derive(Debug)]
pub struct ManifestMmapBuilder {
    entries: Vec<(String, MmapStatEntry)>,
    bloom: Vec<u8>,
}

impl Default for ManifestMmapBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestMmapBuilder {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            bloom: vec![0u8; BLOOM_SIZE],
        }
    }

    /// Add a manifest entry to the builder
    pub fn add_entry(
        &mut self,
        path: &str,
        size: u64,
        mtime: i64,
        mode: u32,
        is_dir: bool,
        is_symlink: bool,
    ) {
        let path_hash = fnv1a_hash(path);
        let flags = if is_dir { 0x01 } else { 0 } | if is_symlink { 0x02 } else { 0 };

        // Add to bloom filter
        let (h1, h2) = bloom_hashes(path);
        let b1 = h1 % (BLOOM_SIZE * 8);
        let b2 = h2 % (BLOOM_SIZE * 8);
        self.bloom[b1 / 8] |= 1 << (b1 % 8);
        self.bloom[b2 / 8] |= 1 << (b2 % 8);

        let entry = MmapStatEntry {
            path_hash,
            size,
            mtime,
            mtime_nsec: 0,
            mode,
            flags,
        };
        self.entries.push((path.to_string(), entry));
    }

    /// Write mmap file to disk (now includes directory indexing)
    pub fn write_to_file(&self, path: &str) -> std::io::Result<()> {
        use std::collections::HashMap;
        use std::io::Write;

        // 1. Group children by parent directory
        let mut dir_map: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        for (idx, (path_str, _entry)) in self.entries.iter().enumerate() {
            let p = std::path::Path::new(path_str);
            if let Some(parent) = p.parent() {
                let parent_str = parent.to_str().unwrap_or("/");
                // Ensure "/" is used for root
                let parent_key = if parent_str.is_empty() {
                    "/"
                } else {
                    parent_str
                };
                dir_map.entry(parent_key.to_string()).or_default().push((
                    p.file_name()
                        .unwrap_or_default()
                        .to_str()
                        .unwrap_or("")
                        .to_string(),
                    idx,
                ));
            }
        }

        // 2. Calculate capacities
        let table_capacity = (self.entries.len() * 2).clamp(1024, MMAP_MAX_ENTRIES);
        let dir_index_capacity = (dir_map.len() * 2).clamp(256, MMAP_MAX_ENTRIES);
        let children_count: usize = dir_map.values().map(|v| v.len()).sum();

        let file_size = mmap_file_size(table_capacity, dir_index_capacity, children_count);

        // 3. Create buffer
        let mut buffer = vec![0u8; file_size];

        // 4. Write header
        let header = ManifestMmapHeader::new(
            self.entries.len() as u32,
            table_capacity as u32,
            dir_index_capacity as u32,
            children_count as u32,
        );
        let header_bytes = unsafe {
            std::slice::from_raw_parts(&header as *const _ as *const u8, ManifestMmapHeader::SIZE)
        };
        buffer[..ManifestMmapHeader::SIZE].copy_from_slice(header_bytes);

        // 5. Write bloom filter
        let bloom_start = header.bloom_offset as usize;
        buffer[bloom_start..bloom_start + BLOOM_SIZE].copy_from_slice(&self.bloom);

        // DEBUG: Check bloom filter content
        let bloom_set_bits: usize = self.bloom.iter().map(|b| b.count_ones() as usize).sum();
        println!(
            "[DEBUG-BUILDER] Bloom filter has {} set bits out of {} total bits",
            bloom_set_bits,
            BLOOM_SIZE * 8
        );
        println!(
            "[DEBUG-BUILDER] Bloom filter first 32 bytes: {:?}",
            &self.bloom[..32]
        );

        // 6. Write stat hash table with linear probing
        // We'll also need a way to map original index to actual slot for dir entries
        let table_start = header.table_offset as usize;
        let mut index_to_slot = vec![0u32; self.entries.len()];

        for (idx, (_path, entry)) in self.entries.iter().enumerate() {
            let start_slot = (entry.path_hash as usize) % table_capacity;
            for i in 0..table_capacity {
                let slot = (start_slot + i) % table_capacity;
                let offset = table_start + slot * MmapStatEntry::SIZE;

                let existing_hash =
                    u64::from_le_bytes(buffer[offset..offset + 8].try_into().unwrap());
                if existing_hash == 0 {
                    let entry_bytes = unsafe {
                        std::slice::from_raw_parts(
                            entry as *const _ as *const u8,
                            MmapStatEntry::SIZE,
                        )
                    };
                    buffer[offset..offset + MmapStatEntry::SIZE].copy_from_slice(entry_bytes);
                    index_to_slot[idx] = slot as u32;
                    break;
                }
            }
        }

        // 7. Write children pool and directory index
        let dir_index_start = header.dir_index_offset as usize;
        let children_start = header.children_offset as usize;
        let mut current_child_idx = 0;

        for (parent_path, children) in dir_map {
            let parent_hash = fnv1a_hash(&parent_path);
            let dir_entry = MmapDirIndexEntry {
                parent_hash,
                children_start: current_child_idx as u32,
                children_count: children.len() as u32,
            };

            // Write to dir index hash table
            let start_slot = (parent_hash as usize) % dir_index_capacity;
            for i in 0..dir_index_capacity {
                let slot = (start_slot + i) % dir_index_capacity;
                let offset = dir_index_start + slot * MmapDirIndexEntry::SIZE;

                let existing_hash =
                    u64::from_le_bytes(buffer[offset..offset + 8].try_into().unwrap());
                if existing_hash == 0 {
                    let entry_bytes = unsafe {
                        std::slice::from_raw_parts(
                            &dir_entry as *const _ as *const u8,
                            MmapDirIndexEntry::SIZE,
                        )
                    };
                    buffer[offset..offset + MmapDirIndexEntry::SIZE].copy_from_slice(entry_bytes);
                    break;
                }
            }

            // Write children to pool
            for (name, stat_idx) in children {
                let mut child = MmapDirChild {
                    name: [0u8; 128],
                    stat_index: index_to_slot[stat_idx],
                    is_dir: if self.entries[stat_idx].1.is_dir() {
                        1
                    } else {
                        0
                    },
                    _pad: [0; 3],
                };
                let name_bytes = name.as_bytes();
                let copy_len = name_bytes.len().min(127);
                child.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

                let offset = children_start + current_child_idx * MmapDirChild::SIZE;
                let entry_bytes = unsafe {
                    std::slice::from_raw_parts(&child as *const _ as *const u8, MmapDirChild::SIZE)
                };
                buffer[offset..offset + MmapDirChild::SIZE].copy_from_slice(entry_bytes);
                current_child_idx += 1;
            }
        }

        // 8. Write atomically
        let temp_path = format!("{}.tmp", path);
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(&buffer)?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)?;

        Ok(())
    }

    /// Get entry count
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Check if daemon is running (socket exists and connectable)
pub fn is_daemon_running() -> bool {
    std::path::Path::new(default_socket_path()).exists()
}

/// IPC Client for communicating with vrift-daemon
#[cfg(feature = "tokio")]
pub mod client {
    use super::*;
    use std::path::Path;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    pub struct DaemonClient {
        stream: UnixStream,
    }

    impl DaemonClient {
        /// Connect to daemon at default socket path
        pub async fn connect() -> anyhow::Result<Self> {
            Self::connect_to(default_socket_path()).await
        }

        /// Connect to daemon at custom socket path
        pub async fn connect_to(socket_path: &str) -> anyhow::Result<Self> {
            let stream = UnixStream::connect(Path::new(socket_path)).await?;
            Ok(Self { stream })
        }

        /// Send a request and receive response
        pub async fn send(&mut self, request: VeloRequest) -> anyhow::Result<VeloResponse> {
            // Serialize request
            let req_bytes = bincode::serialize(&request)?;
            let req_len = (req_bytes.len() as u32).to_le_bytes();

            // Send length + payload
            self.stream.write_all(&req_len).await?;
            self.stream.write_all(&req_bytes).await?;

            // Read response length
            let mut len_buf = [0u8; 4];
            self.stream.read_exact(&mut len_buf).await?;
            let resp_len = u32::from_le_bytes(len_buf) as usize;

            // Read response payload
            let mut resp_buf = vec![0u8; resp_len];
            self.stream.read_exact(&mut resp_buf).await?;

            // Deserialize response
            let response = bincode::deserialize(&resp_buf)?;
            Ok(response)
        }

        /// Handshake with daemon
        pub async fn handshake(&mut self) -> anyhow::Result<String> {
            let request = VeloRequest::Handshake {
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            match self.send(request).await? {
                VeloResponse::HandshakeAck { server_version } => Ok(server_version),
                VeloResponse::Error(e) => anyhow::bail!("Handshake failed: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }

        /// Get daemon status
        pub async fn status(&mut self) -> anyhow::Result<String> {
            match self.send(VeloRequest::Status).await? {
                VeloResponse::StatusAck { status } => Ok(status),
                VeloResponse::Error(e) => anyhow::bail!("Status failed: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = VeloRequest::Status;
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: VeloRequest = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(decoded, VeloRequest::Status));
    }

    #[test]
    fn test_response_serialization() {
        let resp = VeloResponse::StatusAck {
            status: "OK".to_string(),
        };
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: VeloResponse = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(decoded, VeloResponse::StatusAck { .. }));
    }

    #[test]
    fn test_default_socket_path() {
        // Verify default socket path is set
        let path = default_socket_path();
        assert!(!path.is_empty());
        assert!(path.ends_with(".sock"));
    }
}
