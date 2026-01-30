//! # velo-pack
//!
//! Packfile format for Velo Rift hotspot consolidation.
//!
//! Small files cause random I/O. Packfiles consolidate related blobs into
//! sequential storage for improved read performance via OS readahead.
//!
//! ## Design
//!
//! Based on profile-guided packing: files accessed together during startup
//! are packed contiguously.
//!
//! ## Packfile Format
//!
//! ```text
//! +----------------+
//! | Header (32B)   |  Magic, version, entry count
//! +----------------+
//! | Index Table    |  [Hash, Offset, Length] × N
//! +----------------+
//! | Blob Data      |  Raw concatenated blobs
//! +----------------+
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use vrift_cas::Blake3Hash;

/// Magic bytes for packfile identification
const PACK_MAGIC: &[u8; 8] = b"VELOPACK";
/// Current packfile format version
const PACK_VERSION: u32 = 1;

/// Errors that can occur during packfile operations
#[derive(Error, Debug)]
pub enum PackError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("Invalid packfile: {0}")]
    Invalid(String),

    #[error("Blob not found in pack: {hash}")]
    NotFound { hash: String },
}

pub type Result<T> = std::result::Result<T, PackError>;

/// Packfile header (fixed 32 bytes)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackHeader {
    magic: [u8; 8],
    version: u32,
    entry_count: u32,
    index_offset: u64,
    data_offset: u64,
}

impl PackHeader {
    fn new(entry_count: u32, index_offset: u64, data_offset: u64) -> Self {
        Self {
            magic: *PACK_MAGIC,
            version: PACK_VERSION,
            entry_count,
            index_offset,
            data_offset,
        }
    }

    fn validate(&self) -> Result<()> {
        if &self.magic != PACK_MAGIC {
            return Err(PackError::Invalid("Bad magic bytes".to_string()));
        }
        if self.version != PACK_VERSION {
            return Err(PackError::Invalid(format!(
                "Unsupported version: {}",
                self.version
            )));
        }
        Ok(())
    }
}

/// Index entry for a blob in the packfile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackIndexEntry {
    /// BLAKE3 hash of the blob
    pub hash: Blake3Hash,
    /// Offset within the data section
    pub offset: u64,
    /// Length of the blob
    pub length: u64,
}

/// Reader for packfiles
pub struct PackReader {
    path: PathBuf,
    mmap: Mmap,
    index: HashMap<Blake3Hash, PackIndexEntry>,
    data_offset: u64,
}

impl PackReader {
    /// Open a packfile for reading
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let mmap = unsafe { Mmap::map(&file) }.map_err(io::Error::other)?;

        if mmap.len() < 32 {
            return Err(PackError::Invalid("File too small".to_string()));
        }

        // Read header
        let header: PackHeader = bincode::deserialize(&mmap[..32])?;
        header.validate()?;

        // Read index
        let index_start = header.index_offset as usize;
        let index_end = header.data_offset as usize;
        let index_bytes = &mmap[index_start..index_end];
        let entries: Vec<PackIndexEntry> = bincode::deserialize(index_bytes)?;

        let index: HashMap<Blake3Hash, PackIndexEntry> =
            entries.into_iter().map(|e| (e.hash, e)).collect();

        Ok(Self {
            path,
            mmap,
            index,
            data_offset: header.data_offset,
        })
    }

    /// Get a blob by hash (zero-copy via mmap slice)
    pub fn get(&self, hash: &Blake3Hash) -> Result<&[u8]> {
        let entry = self.index.get(hash).ok_or_else(|| PackError::NotFound {
            hash: vrift_cas::CasStore::hash_to_hex(hash),
        })?;

        let start = self.data_offset as usize + entry.offset as usize;
        let end = start + entry.length as usize;

        if end > self.mmap.len() {
            return Err(PackError::Invalid("Blob extends past EOF".to_string()));
        }

        Ok(&self.mmap[start..end])
    }

    /// Check if a blob exists in this packfile
    pub fn contains(&self, hash: &Blake3Hash) -> bool {
        self.index.contains_key(hash)
    }

    /// Get packfile path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get number of blobs in the packfile
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Check if packfile is empty
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Iterate over all hashes in the packfile
    pub fn hashes(&self) -> impl Iterator<Item = &Blake3Hash> {
        self.index.keys()
    }
}

/// Builder for creating new packfiles
pub struct PackWriter {
    output_path: PathBuf,
    entries: Vec<PackIndexEntry>,
    data: Vec<u8>,
}

impl PackWriter {
    /// Create a new packfile writer
    pub fn new<P: AsRef<Path>>(output_path: P) -> Self {
        Self {
            output_path: output_path.as_ref().to_path_buf(),
            entries: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Add a blob to the packfile
    pub fn add(&mut self, hash: Blake3Hash, data: &[u8]) {
        let offset = self.data.len() as u64;
        let length = data.len() as u64;

        self.entries.push(PackIndexEntry {
            hash,
            offset,
            length,
        });

        self.data.extend_from_slice(data);
    }

    /// Write the packfile to disk
    pub fn finish(self) -> Result<PathBuf> {
        let file = File::create(&self.output_path)?;
        let mut writer = BufWriter::new(file);

        // Reserve space for header (will write at end)
        let header_size = 32u64;
        writer.write_all(&[0u8; 32])?;

        // Write index
        let index_offset = header_size;
        let index_bytes = bincode::serialize(&self.entries)?;
        writer.write_all(&index_bytes)?;

        // Write data
        let data_offset = header_size + index_bytes.len() as u64;
        writer.write_all(&self.data)?;

        // Write header at beginning
        let header = PackHeader::new(self.entries.len() as u32, index_offset, data_offset);
        let header_bytes = bincode::serialize(&header)?;

        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(&header_bytes)?;

        writer.flush()?;
        Ok(self.output_path)
    }
}

/// Profile-guided packing: records access order for optimal packing
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AccessProfile {
    /// Blobs in access order
    pub access_order: Vec<Blake3Hash>,
}

impl AccessProfile {
    /// Record a blob access
    pub fn record(&mut self, hash: Blake3Hash) {
        // Deduplicate while preserving first access order
        if !self.access_order.contains(&hash) {
            self.access_order.push(hash);
        }
    }

    /// Save profile to file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        bincode::serialize_into(writer, self)?;
        Ok(())
    }

    /// Load profile from file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let profile = bincode::deserialize_from(reader)?;
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use vrift_cas::CasStore;

    #[test]
    fn test_packfile_roundtrip() {
        let temp = TempDir::new().unwrap();
        let pack_path = temp.path().join("test.pack");

        // Create packfile
        let mut writer = PackWriter::new(&pack_path);

        let data1 = b"Hello, world!";
        let data2 = b"Goodbye, world!";
        let hash1 = CasStore::compute_hash(data1);
        let hash2 = CasStore::compute_hash(data2);

        writer.add(hash1, data1);
        writer.add(hash2, data2);
        writer.finish().unwrap();

        // Read packfile
        let reader = PackReader::open(&pack_path).unwrap();
        assert_eq!(reader.len(), 2);
        assert!(reader.contains(&hash1));
        assert!(reader.contains(&hash2));

        let retrieved1 = reader.get(&hash1).unwrap();
        let retrieved2 = reader.get(&hash2).unwrap();
        assert_eq!(retrieved1, data1);
        assert_eq!(retrieved2, data2);
    }

    #[test]
    fn test_access_profile() {
        let temp = TempDir::new().unwrap();
        let profile_path = temp.path().join("profile.bin");

        let mut profile = AccessProfile::default();
        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];

        profile.record(hash1);
        profile.record(hash2);
        profile.record(hash1); // Duplicate - should be ignored

        assert_eq!(profile.access_order.len(), 2);

        profile.save(&profile_path).unwrap();
        let loaded = AccessProfile::load(&profile_path).unwrap();
        assert_eq!(loaded.access_order.len(), 2);
    }
}
