use rkyv::Archive;
use serde::{Deserialize, Serialize};

/// IPC Protocol Version - bump when making breaking changes
/// v1: Initial protocol with basic requests
/// v2: Added IngestFullScan, RegisterWorkspace (current)
/// v3: New wire format with IpcHeader (magic + request ID)
pub const PROTOCOL_VERSION: u32 = 3;

/// Minimum protocol version this server supports
pub const MIN_PROTOCOL_VERSION: u32 = 1;

// ============================================================================
// IPC Wire Format (v3+)
// ============================================================================

/// Magic number for IPC frames: "VR" (Vrift)
pub const IPC_MAGIC: [u8; 2] = *b"VR";

/// Frame types for IPC protocol (stored in high 4 bits of type_ver byte)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    /// Request from client to server
    Request = 0,
    /// Response from server to client
    Response = 1,
    /// Heartbeat/keepalive
    Heartbeat = 2,
}

impl TryFrom<u8> for FrameType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FrameType::Request),
            1 => Ok(FrameType::Response),
            2 => Ok(FrameType::Heartbeat),
            _ => Err(()),
        }
    }
}

/// Compact IPC Frame Header (8 bytes)
///
/// Wire format:
/// ```text
/// ┌──────────┬────────────┬─────────┬──────────┬──────────┐
/// │Magic (2B)│Type+Ver(1B)│Flags(1B)│Length(2B)│ SeqID(2B)│
/// │  "VR"    │ hi4=type   │reserved │ LE u16   │ LE u16   │
/// │          │ lo4=version│         │ max 64KB │ 0-65535  │
/// └──────────┴────────────┴─────────┴──────────┴──────────┘
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IpcHeader {
    /// Magic number: "VR"
    pub magic: [u8; 2],
    /// Type (high 4 bits) + Protocol Version (low 4 bits)
    pub type_ver: u8,
    /// Flags (reserved for future use)
    pub flags: u8,
    /// Payload length in bytes (max 65535)
    pub length: u16,
    /// Sequence ID for tracing and request-response matching
    pub seq_id: u16,
}

impl IpcHeader {
    /// Size of the header in bytes
    pub const SIZE: usize = 8;

    /// Maximum payload length (64KB - 1)
    pub const MAX_LENGTH: usize = 65535;

    /// Create a new header with specified frame type
    pub fn new(frame_type: FrameType, length: u16, seq_id: u16) -> Self {
        Self {
            magic: IPC_MAGIC,
            type_ver: ((frame_type as u8) << 4) | (PROTOCOL_VERSION as u8 & 0x0F),
            flags: 0,
            length,
            seq_id,
        }
    }

    /// Create a new request header
    pub fn new_request(length: u16, seq_id: u16) -> Self {
        Self::new(FrameType::Request, length, seq_id)
    }

    /// Create a new response header
    pub fn new_response(length: u16, seq_id: u16) -> Self {
        Self::new(FrameType::Response, length, seq_id)
    }

    /// Create a heartbeat header
    pub fn new_heartbeat(seq_id: u16) -> Self {
        Self::new(FrameType::Heartbeat, 0, seq_id)
    }

    /// Validate the header magic
    pub fn is_valid(&self) -> bool {
        self.magic == IPC_MAGIC
    }

    /// Get frame type from high 4 bits
    pub fn frame_type(&self) -> Option<FrameType> {
        FrameType::try_from(self.type_ver >> 4).ok()
    }

    /// Get protocol version from low 4 bits
    pub fn version(&self) -> u8 {
        self.type_ver & 0x0F
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut bytes = [0u8; Self::SIZE];
        bytes[0..2].copy_from_slice(&self.magic);
        bytes[2] = self.type_ver;
        bytes[3] = self.flags;
        bytes[4..6].copy_from_slice(&self.length.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.seq_id.to_le_bytes());
        bytes
    }

    /// Deserialize header from bytes
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Self {
        Self {
            magic: [bytes[0], bytes[1]],
            type_ver: bytes[2],
            flags: bytes[3],
            length: u16::from_le_bytes([bytes[4], bytes[5]]),
            seq_id: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }
}

// ============================================================================
// Frame IO Helpers (v3+ wire format)
// ============================================================================

use std::sync::atomic::{AtomicU16, Ordering};

/// Global sequence ID counter for request tracing
static NEXT_SEQ_ID: AtomicU16 = AtomicU16::new(1);

/// Get next sequence ID (thread-safe, wraps at 65535)
pub fn next_seq_id() -> u16 {
    NEXT_SEQ_ID.fetch_add(1, Ordering::Relaxed)
}

/// Synchronous frame IO (for vrift-shim and blocking contexts)
pub mod frame_sync {
    use super::*;
    use std::io::{Read, Write};

    /// Send a request frame (header + rkyv payload)
    pub fn send_request<W: Write>(writer: &mut W, request: &VeloRequest) -> std::io::Result<u16> {
        let payload = rkyv::to_bytes::<rkyv::rancor::Error>(request)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if payload.len() > IpcHeader::MAX_LENGTH {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "payload too large: {} > {}",
                    payload.len(),
                    IpcHeader::MAX_LENGTH
                ),
            ));
        }

        let seq_id = next_seq_id();
        let header = IpcHeader::new_request(payload.len() as u16, seq_id);

        writer.write_all(&header.to_bytes())?;
        writer.write_all(&payload)?;
        writer.flush()?;

        Ok(seq_id)
    }

    /// Send a response frame
    pub fn send_response<W: Write>(
        writer: &mut W,
        response: &VeloResponse,
        seq_id: u16,
    ) -> std::io::Result<()> {
        let payload = rkyv::to_bytes::<rkyv::rancor::Error>(response)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if payload.len() > IpcHeader::MAX_LENGTH {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "payload too large: {} > {}",
                    payload.len(),
                    IpcHeader::MAX_LENGTH
                ),
            ));
        }

        let header = IpcHeader::new_response(payload.len() as u16, seq_id);

        writer.write_all(&header.to_bytes())?;
        writer.write_all(&payload)?;
        writer.flush()?;

        Ok(())
    }

    /// Read a frame header
    pub fn read_header<R: Read>(reader: &mut R) -> std::io::Result<IpcHeader> {
        let mut buf = [0u8; IpcHeader::SIZE];
        reader.read_exact(&mut buf)?;

        let header = IpcHeader::from_bytes(&buf);
        if !header.is_valid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid IPC magic",
            ));
        }

        Ok(header)
    }

    /// Read frame payload and deserialize as request
    pub fn read_request<R: Read>(reader: &mut R) -> std::io::Result<(IpcHeader, VeloRequest)> {
        let header = read_header(reader)?;

        let mut payload = vec![0u8; header.length as usize];
        reader.read_exact(&mut payload)?;

        let request: VeloRequest =
            rkyv::from_bytes::<VeloRequest, rkyv::rancor::Error>(&payload)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok((header, request))
    }

    /// Read frame payload and deserialize as response
    pub fn read_response<R: Read>(reader: &mut R) -> std::io::Result<(IpcHeader, VeloResponse)> {
        let header = read_header(reader)?;

        let mut payload = vec![0u8; header.length as usize];
        reader.read_exact(&mut payload)?;

        let response: VeloResponse =
            rkyv::from_bytes::<VeloResponse, rkyv::rancor::Error>(&payload)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok((header, response))
    }

    /// Send a heartbeat frame (zero-length payload)
    pub fn send_heartbeat<W: Write>(writer: &mut W) -> std::io::Result<u16> {
        let seq_id = next_seq_id();
        let header = IpcHeader::new(FrameType::Heartbeat, 0, seq_id);

        writer.write_all(&header.to_bytes())?;
        writer.flush()?;

        Ok(seq_id)
    }

    /// Check if received header is a heartbeat
    pub fn is_heartbeat(header: &IpcHeader) -> bool {
        header.frame_type() == Some(FrameType::Heartbeat)
    }
}

/// Async frame IO (for daemon and CLI with tokio)
#[cfg(feature = "tokio")]
pub mod frame_async {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Send a request frame (header + rkyv payload)
    pub async fn send_request<W: AsyncWriteExt + Unpin>(
        writer: &mut W,
        request: &VeloRequest,
    ) -> std::io::Result<u16> {
        let payload = rkyv::to_bytes::<rkyv::rancor::Error>(request)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if payload.len() > IpcHeader::MAX_LENGTH {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "payload too large: {} > {}",
                    payload.len(),
                    IpcHeader::MAX_LENGTH
                ),
            ));
        }

        let seq_id = next_seq_id();
        let header = IpcHeader::new_request(payload.len() as u16, seq_id);

        writer.write_all(&header.to_bytes()).await?;
        writer.write_all(&payload).await?;
        writer.flush().await?;

        Ok(seq_id)
    }

    /// Send a response frame
    pub async fn send_response<W: AsyncWriteExt + Unpin>(
        writer: &mut W,
        response: &VeloResponse,
        seq_id: u16,
    ) -> std::io::Result<()> {
        let payload = rkyv::to_bytes::<rkyv::rancor::Error>(response)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if payload.len() > IpcHeader::MAX_LENGTH {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "payload too large: {} > {}",
                    payload.len(),
                    IpcHeader::MAX_LENGTH
                ),
            ));
        }

        let header = IpcHeader::new_response(payload.len() as u16, seq_id);

        writer.write_all(&header.to_bytes()).await?;
        writer.write_all(&payload).await?;
        writer.flush().await?;

        Ok(())
    }

    /// Read a frame header
    pub async fn read_header<R: AsyncReadExt + Unpin>(
        reader: &mut R,
    ) -> std::io::Result<IpcHeader> {
        let mut buf = [0u8; IpcHeader::SIZE];
        reader.read_exact(&mut buf).await?;

        let header = IpcHeader::from_bytes(&buf);
        if !header.is_valid() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid IPC magic",
            ));
        }

        Ok(header)
    }

    /// Read frame payload and deserialize as request
    pub async fn read_request<R: AsyncReadExt + Unpin>(
        reader: &mut R,
    ) -> std::io::Result<(IpcHeader, VeloRequest)> {
        let header = read_header(reader).await?;

        let mut payload = vec![0u8; header.length as usize];
        reader.read_exact(&mut payload).await?;

        let request: VeloRequest =
            rkyv::from_bytes::<VeloRequest, rkyv::rancor::Error>(&payload)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok((header, request))
    }

    /// Read frame payload and deserialize as response
    pub async fn read_response<R: AsyncReadExt + Unpin>(
        reader: &mut R,
    ) -> std::io::Result<(IpcHeader, VeloResponse)> {
        let header = read_header(reader).await?;

        let mut payload = vec![0u8; header.length as usize];
        reader.read_exact(&mut payload).await?;

        let response: VeloResponse =
            rkyv::from_bytes::<VeloResponse, rkyv::rancor::Error>(&payload)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok((header, response))
    }

    // ========================================================================
    // Timeout Wrappers
    // ========================================================================

    /// Default read timeout (30 seconds)
    pub const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    /// Default write timeout (10 seconds)
    pub const DEFAULT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    /// Send request with timeout
    pub async fn send_request_timeout<W: AsyncWriteExt + Unpin>(
        writer: &mut W,
        request: &VeloRequest,
        timeout: std::time::Duration,
    ) -> std::io::Result<u16> {
        tokio::time::timeout(timeout, send_request(writer, request))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "send request timeout")
            })?
    }

    /// Read response with timeout
    pub async fn read_response_timeout<R: AsyncReadExt + Unpin>(
        reader: &mut R,
        timeout: std::time::Duration,
    ) -> std::io::Result<(IpcHeader, VeloResponse)> {
        tokio::time::timeout(timeout, read_response(reader))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "read response timeout")
            })?
    }

    /// Read request with timeout (for daemon)
    pub async fn read_request_timeout<R: AsyncReadExt + Unpin>(
        reader: &mut R,
        timeout: std::time::Duration,
    ) -> std::io::Result<(IpcHeader, VeloRequest)> {
        tokio::time::timeout(timeout, read_request(reader))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "read request timeout")
            })?
    }

    // ========================================================================
    // Heartbeat
    // ========================================================================

    /// Send a heartbeat frame (zero-length payload)
    pub async fn send_heartbeat<W: AsyncWriteExt + Unpin>(writer: &mut W) -> std::io::Result<u16> {
        let seq_id = next_seq_id();
        let header = IpcHeader::new(FrameType::Heartbeat, 0, seq_id);

        writer.write_all(&header.to_bytes()).await?;
        writer.flush().await?;

        Ok(seq_id)
    }

    /// Check if received header is a heartbeat
    pub fn is_heartbeat(header: &IpcHeader) -> bool {
        header.frame_type() == Some(FrameType::Heartbeat)
    }
}

#[derive(Debug, Serialize, Deserialize, Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum VeloRequest {
    Handshake {
        client_version: String,
        /// Protocol version (required in v3+)
        protocol_version: u32,
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
    /// Full scan ingest request (CLI → vDird)
    /// CLI becomes thin client, vDird handles all ingest logic
    IngestFullScan {
        /// Path to ingest (directory)
        path: String,
        /// Output manifest path
        manifest_path: String,
        /// CAS root directory (TheSource)
        cas_root: String,
        /// Number of threads (None = auto)
        threads: Option<usize>,
        /// Use Phantom mode (move instead of link)
        phantom: bool,
        /// Use Tier-1 mode (immutable)
        tier1: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[cfg(feature = "manifest")]
pub use vrift_manifest::VnodeEntry;

#[cfg(not(feature = "manifest"))]
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct VnodeEntry {
    pub content_hash: [u8; 32],
    pub size: u64,
    pub mtime: u64,
    pub mode: u32,
    pub flags: u16,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub _pad: u16,
}

#[cfg(not(feature = "manifest"))]
impl VnodeEntry {
    pub fn is_dir(&self) -> bool {
        (self.flags & 1) != 0
    }
}

// ============================================================================
// Structured Error Types (Phase 3: IPC Error Semantics)
// ============================================================================

/// Error categories for IPC responses
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub enum VeloErrorKind {
    /// Resource not found (file, entry, workspace)
    NotFound,
    /// Permission denied (UID mismatch, access control)
    PermissionDenied,
    /// Invalid path (traversal, malformed)
    InvalidPath,
    /// Workspace not registered
    WorkspaceNotRegistered,
    /// Ingest operation failed
    IngestFailed,
    /// I/O error (disk, network)
    IoError,
    /// Lock acquisition failed (EWOULDBLOCK)
    LockFailed,
    /// Internal server error
    Internal,
}

/// Structured error for IPC responses
#[derive(Debug, Clone, Serialize, Deserialize, Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct VeloError {
    /// Error category
    pub kind: VeloErrorKind,
    /// Human-readable error message
    pub message: String,
    /// Optional path associated with the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl VeloError {
    /// Create a new error
    pub fn new(kind: VeloErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            path: None,
        }
    }

    /// Create error with associated path
    pub fn with_path(
        kind: VeloErrorKind,
        message: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            path: Some(path.into()),
        }
    }

    // Convenience constructors for common errors

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(VeloErrorKind::NotFound, message)
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(VeloErrorKind::PermissionDenied, message)
    }

    pub fn invalid_path(message: impl Into<String>) -> Self {
        Self::new(VeloErrorKind::InvalidPath, message)
    }

    pub fn workspace_not_registered() -> Self {
        Self::new(
            VeloErrorKind::WorkspaceNotRegistered,
            "Workspace not registered",
        )
    }

    pub fn io_error(message: impl Into<String>) -> Self {
        Self::new(VeloErrorKind::IoError, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(VeloErrorKind::Internal, message)
    }

    /// Set path on an existing error (builder pattern)
    pub fn set_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Get CLI exit code for this error kind
    ///
    /// Uses standard Unix exit code conventions:
    /// - 1: General error (Internal, IoError)
    /// - 2: Not found (NotFound, WorkspaceNotRegistered)
    /// - 22: Invalid argument (InvalidPath)
    /// - 77: Permission denied (PermissionDenied)
    /// - 78: Lock failure (LockFailed)
    /// - 79: Ingest failure (IngestFailed)
    pub fn exit_code(&self) -> i32 {
        match self.kind {
            VeloErrorKind::NotFound => 2,
            VeloErrorKind::WorkspaceNotRegistered => 2,
            VeloErrorKind::InvalidPath => 22,
            VeloErrorKind::PermissionDenied => 77,
            VeloErrorKind::LockFailed => 78,
            VeloErrorKind::IngestFailed => 79,
            VeloErrorKind::IoError => 1,
            VeloErrorKind::Internal => 1,
        }
    }
}

impl std::fmt::Display for VeloError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{:?}: {} (path: {})", self.kind, self.message, path)
        } else {
            write!(f, "{:?}: {}", self.kind, self.message)
        }
    }
}

impl std::error::Error for VeloError {}

#[derive(Debug, Serialize, Deserialize, Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum VeloResponse {
    HandshakeAck {
        server_version: String,
        /// Server protocol version
        protocol_version: u32,
        /// Whether client version is compatible
        compatible: bool,
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
    /// Ingest completion acknowledgement
    IngestAck {
        /// Total files processed
        files: u64,
        /// Unique blobs stored
        blobs: u64,
        /// New bytes stored
        new_bytes: u64,
        /// Total bytes processed
        total_bytes: u64,
        /// Duration in milliseconds
        duration_ms: u64,
        /// Manifest path
        manifest_path: String,
    },
    /// Structured error response (Phase 3: replaces Error(String))
    Error(VeloError),
}

/// Check if a protocol version is compatible with this build
pub fn is_version_compatible(client_version: u32) -> bool {
    (MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&client_version)
}

/// Default socket path (internal fallback for DaemonClient)
/// Prefer using vrift_config::config().socket_path() when available
const DEFAULT_SOCKET_PATH: &str = "/tmp/vrift.sock";

/// Get default socket path (for internal use only)
fn default_socket_path() -> &'static str {
    DEFAULT_SOCKET_PATH
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

        /// Send a request and receive response using v3 frame protocol
        pub async fn send(&mut self, request: VeloRequest) -> anyhow::Result<VeloResponse> {
            use crate::frame_async;

            // Send request frame
            let seq_id = frame_async::send_request(&mut self.stream, &request).await?;

            // Read response frame
            let (header, response) = frame_async::read_response(&mut self.stream).await?;

            // Verify seq_id matches (optional but good for debugging)
            if header.seq_id != seq_id {
                anyhow::bail!(
                    "Response seq_id mismatch: expected {}, got {}",
                    seq_id,
                    header.seq_id
                );
            }

            Ok(response)
        }

        /// Handshake with daemon
        pub async fn handshake(&mut self) -> anyhow::Result<String> {
            let request = VeloRequest::Handshake {
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: PROTOCOL_VERSION,
            };
            match self.send(request).await? {
                VeloResponse::HandshakeAck {
                    server_version,
                    compatible,
                    ..
                } => {
                    if !compatible {
                        anyhow::bail!("Protocol version mismatch");
                    }
                    Ok(server_version)
                }
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
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&req).unwrap();
        let decoded: VeloRequest =
            rkyv::from_bytes::<VeloRequest, rkyv::rancor::Error>(&bytes).unwrap();
        assert!(matches!(decoded, VeloRequest::Status));
    }

    #[test]
    fn test_response_serialization() {
        let resp = VeloResponse::StatusAck {
            status: "OK".to_string(),
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&resp).unwrap();
        let decoded: VeloResponse =
            rkyv::from_bytes::<VeloResponse, rkyv::rancor::Error>(&bytes).unwrap();
        assert!(matches!(decoded, VeloResponse::StatusAck { .. }));
    }

    #[test]
    fn test_default_socket_path() {
        // Verify default socket path is set
        let path = default_socket_path();
        assert!(!path.is_empty());
        assert!(path.ends_with(".sock"));
    }

    #[test]
    fn test_ipc_header_size() {
        // Verify header is exactly 8 bytes
        assert_eq!(IpcHeader::SIZE, 8);
        assert_eq!(std::mem::size_of::<IpcHeader>(), 8);
    }

    #[test]
    fn test_ipc_header_roundtrip() {
        let header = IpcHeader::new_request(1234, 42);
        let bytes = header.to_bytes();
        let decoded = IpcHeader::from_bytes(&bytes);

        assert_eq!(decoded.magic, IPC_MAGIC);
        assert_eq!(decoded.length, 1234);
        assert_eq!(decoded.frame_type(), Some(FrameType::Request));
        assert_eq!(decoded.seq_id, 42);
        assert_eq!(decoded.version(), PROTOCOL_VERSION as u8);
        assert!(decoded.is_valid());
    }

    #[test]
    fn test_ipc_header_types() {
        let req = IpcHeader::new_request(100, 1);
        assert_eq!(req.frame_type(), Some(FrameType::Request));
        assert_eq!(req.version(), 3); // PROTOCOL_VERSION

        let resp = IpcHeader::new_response(200, 2);
        assert_eq!(resp.frame_type(), Some(FrameType::Response));

        let hb = IpcHeader::new_heartbeat(3);
        assert_eq!(hb.frame_type(), Some(FrameType::Heartbeat));
        assert_eq!(hb.length, 0);
    }

    #[test]
    fn test_ipc_header_invalid_magic() {
        let mut bytes = IpcHeader::new_request(100, 1).to_bytes();
        bytes[0] = b'X'; // corrupt magic
        let decoded = IpcHeader::from_bytes(&bytes);
        assert!(!decoded.is_valid());
    }

    #[test]
    fn test_ipc_header_max_length() {
        // Test max payload length (64KB - 1)
        let header = IpcHeader::new_request(65535, 0);
        assert_eq!(header.length, 65535);
        assert_eq!(IpcHeader::MAX_LENGTH, 65535);
    }

    #[test]
    fn test_version_compatibility() {
        // v0 is NOT valid (legacy compat removed in v3)
        assert!(!is_version_compatible(0));
        // v1 is supported (MIN_PROTOCOL_VERSION)
        assert!(is_version_compatible(1));
        // v2 is supported
        assert!(is_version_compatible(2));
        // v3 is current (PROTOCOL_VERSION)
        assert!(is_version_compatible(3));
        // v4 is not yet supported
        assert!(!is_version_compatible(4));
        // Very high version not supported
        assert!(!is_version_compatible(100));
    }

    #[test]
    fn test_frame_sync_roundtrip() {
        use crate::frame_sync;
        use std::io::Cursor;

        // Test request roundtrip
        let request = VeloRequest::Status;
        let mut buf = Vec::new();
        let seq_id = frame_sync::send_request(&mut buf, &request).unwrap();

        let mut cursor = Cursor::new(&buf);
        let (header, decoded) = frame_sync::read_request(&mut cursor).unwrap();

        assert_eq!(header.seq_id, seq_id);
        assert_eq!(header.frame_type(), Some(FrameType::Request));
        assert!(matches!(decoded, VeloRequest::Status));
    }

    #[test]
    fn test_frame_sync_response_roundtrip() {
        use crate::frame_sync;
        use std::io::Cursor;

        let response = VeloResponse::StatusAck {
            status: "OK".to_string(),
        };
        let mut buf = Vec::new();
        frame_sync::send_response(&mut buf, &response, 42).unwrap();

        let mut cursor = Cursor::new(&buf);
        let (header, decoded) = frame_sync::read_response(&mut cursor).unwrap();

        assert_eq!(header.seq_id, 42);
        assert_eq!(header.frame_type(), Some(FrameType::Response));
        assert!(matches!(decoded, VeloResponse::StatusAck { .. }));
    }

    // =========================================================================
    // VeloError Tests
    // =========================================================================

    #[test]
    fn test_velo_error_serialization() {
        let error = VeloError::not_found("Resource not found");
        let json = serde_json::to_string(&error).unwrap();
        let decoded: VeloError = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.kind, VeloErrorKind::NotFound);
        assert_eq!(decoded.message, "Resource not found");
        assert!(decoded.path.is_none());
    }

    #[test]
    fn test_velo_error_with_path() {
        let error = VeloError::with_path(
            VeloErrorKind::NotFound,
            "File not found",
            "/path/to/file.txt",
        );
        let json = serde_json::to_string(&error).unwrap();
        let decoded: VeloError = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.path, Some("/path/to/file.txt".to_string()));
    }

    #[test]
    fn test_velo_error_set_path() {
        let error = VeloError::not_found("File not found").set_path("/some/path.txt");

        assert_eq!(error.path, Some("/some/path.txt".to_string()));
    }

    #[test]
    fn test_velo_error_display() {
        let error = VeloError::permission_denied("Access denied");
        let display = format!("{}", error);

        assert!(display.contains("PermissionDenied"));
        assert!(display.contains("Access denied"));
    }

    #[test]
    fn test_velo_error_display_with_path() {
        let error =
            VeloError::with_path(VeloErrorKind::InvalidPath, "Path traversal", "/etc/passwd");
        let display = format!("{}", error);

        assert!(display.contains("InvalidPath"));
        assert!(display.contains("/etc/passwd"));
    }

    #[test]
    fn test_velo_error_exit_codes() {
        assert_eq!(VeloError::not_found("").exit_code(), 2);
        assert_eq!(VeloError::workspace_not_registered().exit_code(), 2);
        assert_eq!(VeloError::invalid_path("").exit_code(), 22);
        assert_eq!(VeloError::permission_denied("").exit_code(), 77);
        assert_eq!(
            VeloError::new(VeloErrorKind::LockFailed, "").exit_code(),
            78
        );
        assert_eq!(
            VeloError::new(VeloErrorKind::IngestFailed, "").exit_code(),
            79
        );
        assert_eq!(VeloError::io_error("").exit_code(), 1);
        assert_eq!(VeloError::internal("").exit_code(), 1);
    }

    #[test]
    fn test_velo_error_response_serialization() {
        let response = VeloResponse::Error(VeloError::not_found("Not found"));
        let json = serde_json::to_string(&response).unwrap();
        let decoded: VeloResponse = serde_json::from_str(&json).unwrap();

        if let VeloResponse::Error(err) = decoded {
            assert_eq!(err.kind, VeloErrorKind::NotFound);
        } else {
            panic!("Expected VeloResponse::Error");
        }
    }
}
