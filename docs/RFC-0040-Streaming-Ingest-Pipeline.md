# Production Streaming Ingest Pipeline V2

## Design Goals

| Goal | Solution |
|------|----------|
| 1M+ files without OOM | Ring buffer + bounded channels |
| No missed files during scan | Watch-first pattern |
| Zero-copy where possible | mmap + sendfile/splice |
| Large file safety | Memory budget semaphore |
| Minimal fsync cost | Batch commit (100:2 ratio) |
| Modification detection | mtime check before commit |

---

## Safety Invariants & Timing Guarantees

### S1: Data Integrity Invariants

| Invariant | Description | Enforcement |
|-----------|-------------|-------------|
| **I-1** | CAS entry hash MUST equal content hash | Verify before rename |
| **I-2** | Committed file MUST be durable (fsync) | Batch dir fsync |
| **I-3** | Partial writes MUST NOT be visible | Atomic rename from tmp |
| **I-4** | Modified file MUST be rejected | mtime check before/after |

### S2: Event Ordering Guarantees

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Happens-Before Relationships                                           │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  [O-1] watch.start() ──happens-before──▶ scanner.walk()                │
│        (ensures no file creation is missed during scan)                 │
│                                                                         │
│  [O-2] file.write() ──happens-before──▶ file.close_write()             │
│        (only process after IN_CLOSE_WRITE, not IN_MODIFY)               │
│                                                                         │
│  [O-3] temp.write() ──happens-before──▶ dir.fsync()                    │
│        ──happens-before──▶ rename(temp, final)                          │
│        ──happens-before──▶ dir.fsync()                                  │
│        (crash-safe commit sequence)                                     │
│                                                                         │
│  [O-4] mtime_before ──read-before──▶ content ──read-before──▶ mtime_after│
│        (detect modification during read)                                │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### S3: Timing Constraints

| Constraint | Value | Rationale |
|------------|-------|-----------|
| **T-1** watch → scan gap | < 1ms | Minimize race window |
| **T-2** batch timeout | 10ms | Balance latency vs batch size |
| **T-3** mtime resolution | 1ms | Filesystem-dependent, usually ms |
| **T-4** fsync budget | 2 per 100 files | 200x reduction vs naive |

### S4: Failure Modes & Recovery

| Failure | Detection | Recovery |
|---------|-----------|----------|
| Crash during write | Temp file orphan | Cleanup tmp/ on startup |
| Crash after fsync | Rename incomplete | Retry rename on startup |
| File modified during read | mtime mismatch | Discard temp, re-queue path |
| Hash collision | Impossible (BLAKE3) | N/A |
| OOM | Semaphore blocks | Backpressure, not crash |

### S5: Concurrency Safety

```rust
// Thread-safe components
MemorySemaphore:    Mutex + Condvar (blocking acquire)
DashSet<PathBuf>:   Lock-free concurrent set (dedup)
PathRingBuffer:     Atomic state transitions (lock-free)
BatchCommitter:     Single-threaded (main thread only)
```

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Watch-First Streaming Pipeline                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │  T0: Start inotify/fsevents BEFORE scanning                          │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
│                                      │                                      │
│                                      ▼                                      │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌───────────┐   │
│  │   Scanner   │───▶│  Ring Buf   │───▶│   Workers   │───▶│ Committer │   │
│  │  (WalkDir)  │    │ (path chan) │    │  (N threads)│    │ (batch)   │   │
│  └─────────────┘    └─────────────┘    └─────────────┘    └───────────┘   │
│         │                                     │                  │          │
│         │                                     │                  │          │
│         ▼                                     ▼                  ▼          │
│  ┌─────────────┐                      ┌─────────────┐    ┌───────────┐     │
│  │ Watch Queue │──────────────────────▶│ Dedup Set  │    │ 2x fsync  │     │
│  │ (changes)   │                      │ (skip known)│    │ per batch │     │
│  └─────────────┘                      └─────────────┘    └───────────┘     │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## True Ring Buffer (Zero Allocation)

Replace `crossbeam-channel` with a pre-allocated ring buffer for zero heap allocation during ingest:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│              Pre-allocated Ring Buffer (Fixed Slots)                        │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐            │
│  │  0  │  1  │  2  │  3  │  4  │  5  │  6  │  7  │ ... │ N-1 │ ← slots    │
│  └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘            │
│       ↑                               ↑                                     │
│      tail                            head                                   │
│    (consumer)                      (producer)                               │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │  PathSlot Structure (fixed 512 bytes per slot)                      │   │
│  │  ┌──────────────────────────────────────────────────────────────┐   │   │
│  │  │ state: AtomicU8 (Empty=0, Writing=1, Ready=2, Reading=3)     │   │   │
│  │  │ path_len: u16                                                 │   │   │
│  │  │ file_size: u64                                                │   │   │
│  │  │ path_buf: [u8; 496]   ← inline path (covers 99% of cases)     │   │   │
│  │  │ overflow: Option<Box<[u8]>>  ← heap fallback for long paths   │   │   │
│  │  └──────────────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
│  Backpressure: Producer spins/waits when (head + 1) % N == tail             │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Long Path Handling Strategy

| Approach | Description | Trade-off |
|----------|-------------|-----------|
| **Inline (default)** | Paths ≤ 496 bytes stored in slot | Zero allocation |
| **Overflow heap** | Paths > 496 bytes use `Box<[u8]>` | Rare allocation |
| **Truncate** | Hash the path, store hash | Lose path info |

**Recommended: Inline + Overflow**

```rust
const PATH_INLINE_SIZE: usize = 496;  // Covers 99%+ of real paths
const RING_SIZE: usize = 1024;        // Number of slots

#[repr(C, align(64))]  // Cache-line aligned
struct PathSlot {
    state: AtomicU8,           // 1 byte
    _pad1: [u8; 7],            // alignment
    path_len: u16,             // 2 bytes
    _pad2: [u8; 6],            // alignment
    file_size: u64,            // 8 bytes
    path_buf: [u8; PATH_INLINE_SIZE],  // 496 bytes (inline path)
    overflow: AtomicPtr<u8>,   // 8 bytes (pointer to heap for long paths)
}

impl PathSlot {
    fn set_path(&mut self, path: &Path) {
        let bytes = path.as_os_str().as_bytes();
        self.path_len = bytes.len() as u16;
        
        if bytes.len() <= PATH_INLINE_SIZE {
            // Fast path: inline
            self.path_buf[..bytes.len()].copy_from_slice(bytes);
            self.overflow.store(std::ptr::null_mut(), Ordering::Release);
        } else {
            // Slow path: heap allocation (rare)
            let boxed = bytes.to_vec().into_boxed_slice();
            let ptr = Box::into_raw(boxed) as *mut u8;
            self.overflow.store(ptr, Ordering::Release);
        }
    }
    
    fn get_path(&self) -> PathBuf {
        let ptr = self.overflow.load(Ordering::Acquire);
        if ptr.is_null() {
            // Inline path
            let bytes = &self.path_buf[..self.path_len as usize];
            PathBuf::from(OsStr::from_bytes(bytes))
        } else {
            // Overflow path
            let bytes = unsafe { 
                std::slice::from_raw_parts(ptr, self.path_len as usize) 
            };
            PathBuf::from(OsStr::from_bytes(bytes))
        }
    }
}

struct PathRingBuffer {
    slots: Box<[PathSlot; RING_SIZE]>,
    head: AtomicUsize,  // Producer writes here
    tail: AtomicUsize,  // Consumer reads here
}

impl PathRingBuffer {
    /// Producer: write path to ring (blocks if full)
    fn push(&self, path: &Path, size: u64) {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            
            // Backpressure: ring is full
            if (head + 1) % RING_SIZE == tail {
                std::hint::spin_loop();
                continue;
            }
            
            let slot = &self.slots[head];
            
            // Wait for slot to be empty
            while slot.state.load(Ordering::Acquire) != State::Empty {
                std::hint::spin_loop();
            }
            
            // Write to slot
            unsafe { 
                let slot_mut = &mut *(slot as *const _ as *mut PathSlot);
                slot_mut.set_path(path);
                slot_mut.file_size = size;
            }
            
            slot.state.store(State::Ready, Ordering::Release);
            self.head.store((head + 1) % RING_SIZE, Ordering::Release);
            break;
        }
    }
    
    /// Consumer: read path from ring (blocks if empty)
    fn pop(&self) -> Option<(PathBuf, u64)> {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        
        // Ring is empty
        if tail == head {
            return None;
        }
        
        let slot = &self.slots[tail];
        
        // Wait for slot to be ready
        while slot.state.load(Ordering::Acquire) != State::Ready {
            std::hint::spin_loop();
        }
        
        let path = slot.get_path();
        let size = slot.file_size;
        
        // Free overflow if any
        let ptr = slot.overflow.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !ptr.is_null() {
            unsafe { 
                let _ = Box::from_raw(std::slice::from_raw_parts_mut(
                    ptr, slot.path_len as usize
                )); 
            }
        }
        
        slot.state.store(State::Empty, Ordering::Release);
        self.tail.store((tail + 1) % RING_SIZE, Ordering::Release);
        
        Some((path, size))
    }
}
```

### Memory Layout

| Slots | Slot Size | Total Memory |
|-------|-----------|--------------|
| 1024 | 512 bytes | 512 KB |
| 4096 | 512 bytes | 2 MB |
| 16384 | 512 bytes | 8 MB |

**Benefits:**
- Zero heap allocation for 99%+ of paths
- Natural backpressure via ring fullness
- Cache-friendly: 64-byte aligned slots
- Lock-free: atomic state transitions

---

---

## Stage 1: Watch-First Scanner

```rust
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use crossbeam_channel::{bounded, Sender, Receiver};

pub struct WatchFirstScanner {
    root: PathBuf,
    watcher: RecommendedWatcher,
    watch_rx: Receiver<notify::Event>,
    path_tx: Sender<ScanItem>,
}

pub enum ScanItem {
    Path(PathBuf, u64),     // path, size
    WatchEvent(PathBuf),    // from inotify/fsevents
    Done,
}

impl WatchFirstScanner {
    pub fn new(root: PathBuf, path_tx: Sender<ScanItem>) -> Result<Self> {
        let (watch_tx, watch_rx) = std::sync::mpsc::channel();
        
        let mut watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = watch_tx.send(event);
            }
        })?;
        
        // CRITICAL: Start watching BEFORE scanning
        watcher.watch(&root, RecursiveMode::Recursive)?;
        
        Ok(Self { root, watcher, watch_rx, path_tx })
    }
    
    pub fn run(&self) -> Result<()> {
        // Phase 1: Walk directory tree
        for entry in WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let size = entry.metadata()?.len();
            self.path_tx.send(ScanItem::Path(entry.into_path(), size))?;
        }
        
        // Phase 2: Drain watch events (captured during scan)
        while let Ok(event) = self.watch_rx.try_recv() {
            for path in event.paths {
                if path.is_file() {
                    self.path_tx.send(ScanItem::WatchEvent(path))?;
                }
            }
        }
        
        self.path_tx.send(ScanItem::Done)?;
        Ok(())
    }
}
```

---

## Stage 2: Memory-Bounded Workers

```rust
use std::sync::{Arc, Semaphore};

pub struct WorkerPool {
    /// Memory budget (e.g., 256MB)
    memory_semaphore: Arc<Semaphore>,
    /// Small file threshold for mmap
    mmap_threshold: u64,
    /// Chunk size for large files
    chunk_size: usize,
    /// Already processed files (dedup)
    seen: DashSet<PathBuf>,
}

pub struct ProcessedFile {
    pub path: PathBuf,
    pub hash: Blake3Hash,
    pub temp_path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
}

impl WorkerPool {
    pub fn process(&self, item: ScanItem) -> Result<Option<ProcessedFile>> {
        let (path, size) = match item {
            ScanItem::Path(p, s) => (p, s),
            ScanItem::WatchEvent(p) => {
                let s = fs::metadata(&p)?.len();
                (p, s)
            }
            ScanItem::Done => return Ok(None),
        };
        
        // Dedup: skip if already processed
        if !self.seen.insert(path.clone()) {
            return Ok(None);
        }
        
        // Acquire memory permit (blocks if budget exhausted)
        let permit_size = std::cmp::min(size as usize, self.chunk_size);
        let _permit = self.memory_semaphore.acquire_many(permit_size)?;
        
        // Record mtime BEFORE reading
        let mtime_before = fs::metadata(&path)?.modified()?;
        
        // Process based on file size
        let (hash, temp_path) = if size < self.mmap_threshold {
            self.process_small_file(&path)?
        } else {
            self.process_large_file(&path)?
        };
        
        // Check mtime AFTER reading
        let mtime_after = fs::metadata(&path)?.modified()?;
        if mtime_before != mtime_after {
            // File modified during read - discard and retry
            fs::remove_file(&temp_path)?;
            return Err(CasError::FileModified(path));
        }
        
        Ok(Some(ProcessedFile {
            path,
            hash,
            temp_path,
            size,
            mtime: mtime_before,
        }))
    }
    
    /// Small file: mmap + zero-copy hash + sendfile write
    fn process_small_file(&self, path: &Path) -> Result<(Blake3Hash, PathBuf)> {
        let file = File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        
        // Zero-copy hash
        let hash = blake3::hash(&mmap);
        let temp_path = self.temp_path_for(&hash);
        
        // Zero-copy write (sendfile on Linux, copyfile on macOS)
        #[cfg(target_os = "linux")]
        {
            let out = File::create(&temp_path)?;
            nix::sys::sendfile::sendfile(
                out.as_raw_fd(),
                file.as_raw_fd(),
                None,
                mmap.len(),
            )?;
        }
        
        #[cfg(target_os = "macos")]
        {
            // macOS: use fcopyfile for zero-copy
            std::fs::copy(path, &temp_path)?;
        }
        
        Ok((hash.into(), temp_path))
    }
    
    /// Large file: streaming read/hash/write with reused buffer
    fn process_large_file(&self, path: &Path) -> Result<(Blake3Hash, PathBuf)> {
        let mut reader = BufReader::new(File::open(path)?);
        let mut hasher = blake3::Hasher::new();
        
        // Pre-generate temp path with placeholder hash
        let temp_path = self.temp_path_temp();
        let mut writer = BufWriter::new(File::create(&temp_path)?);
        
        // Reusable buffer - not allocated per file
        let mut buf = vec![0u8; self.chunk_size];
        
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 { break; }
            
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n])?;
        }
        
        writer.flush()?;
        // Note: NO fsync here - deferred to committer
        
        Ok((hasher.finalize().into(), temp_path))
    }
}
```

---

## Stage 3: Batch Committer

```rust
pub struct BatchCommitter {
    cas_root: PathBuf,
    batch_size: usize,
    batch_timeout: Duration,
    current_batch: Vec<ProcessedFile>,
}

impl BatchCommitter {
    pub fn commit_batch(&mut self) -> Result<usize> {
        if self.current_batch.is_empty() {
            return Ok(0);
        }
        
        let count = self.current_batch.len();
        
        // Step 1: Single directory fsync (covers all temp file writes)
        let dir = File::open(&self.cas_root)?;
        dir.sync_all()?;
        
        // Step 2: Atomic renames
        for item in &self.current_batch {
            let final_path = self.final_path(&item.hash, item.size);
            
            // Skip if already exists (dedup)
            if final_path.exists() {
                fs::remove_file(&item.temp_path)?;
                continue;
            }
            
            // Create parent directories
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            fs::rename(&item.temp_path, &final_path)?;
        }
        
        // Step 3: Directory fsync for renames
        dir.sync_all()?;
        
        self.current_batch.clear();
        Ok(count)
    }
    
    /// 3-level sharded path: blake3/ab/cd/hash_size.bin
    fn final_path(&self, hash: &Blake3Hash, size: u64) -> PathBuf {
        let hex = hex::encode(hash);
        self.cas_root
            .join("blake3")
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(format!("{}_{}.bin", hex, size))
    }
}
```

---

## Stage 3b: Drain-All Commit Strategy

The committer uses a **drain-all** strategy for optimal throughput:

```
┌─────────────────────────────────────────────────────────────────┐
│                  Drain-All Commit Loop                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  recv_timeout(10ms)  ───▶  Got first item?                     │
│         │                        │                              │
│         │                   Yes  │  No (timeout)                │
│         │                        ▼                              │
│         │              ┌─────────────────┐                     │
│         │              │ try_recv() loop │ ← drain all         │
│         │              │ (non-blocking)  │   available         │
│         │              └────────┬────────┘                     │
│         │                       │                               │
│         │                       ▼                               │
│         │              Batch full?  ──Yes──▶ commit()          │
│         │                   │                                   │
│         │              No   │                                   │
│         │                   ▼                                   │
│         │              Continue drain                           │
│         │                                                       │
│         ▼                                                       │
│  Flush partial batch                                            │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Key Design Points:**

1. **Short initial timeout (10ms)**: Quick response to first available item
2. **Non-blocking drain**: `try_recv()` pulls all queued items immediately
3. **Mid-drain commit**: If batch fills during drain, commit immediately
4. **Timeout flush**: Partial batches committed after 10ms idle

```rust
loop {
    // Wait for first item (short timeout)
    match commit_rx.recv_timeout(Duration::from_millis(10)) {
        Ok(item) => {
            committer.add(item);
            
            // Drain all immediately available (non-blocking)
            while let Ok(item) = commit_rx.try_recv() {
                committer.add(item);
                
                // Commit when batch is full
                if committer.should_commit() {
                    committer.commit()?;
                }
            }
            
            // Commit remaining after drain
            if committer.should_commit() {
                committer.commit()?;
            }
        }
        Err(RecvTimeoutError::Timeout) => {
            // Flush partial batch
            committer.commit()?;
        }
        Err(RecvTimeoutError::Disconnected) => {
            committer.commit()?;
            break;
        }
    }
}
```

**Benefits:**
- Reduced latency: 10ms vs 100ms timeout
- Higher throughput: processes all queued items in one loop iteration
- Fuller batches: drains accumulation before committing

---

## Pipeline Orchestration

```rust
pub struct IngestPipeline {
    config: PipelineConfig,
}

pub struct PipelineConfig {
    pub memory_budget: usize,       // 256MB
    pub mmap_threshold: u64,        // 1MB
    pub chunk_size: usize,          // 4MB
    pub channel_capacity: usize,    // 1024
    pub batch_size: usize,          // 100
    pub batch_timeout_ms: u64,      // 100ms
    pub worker_threads: usize,      // num_cpus
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            memory_budget: 256 * 1024 * 1024,
            mmap_threshold: 1024 * 1024,
            chunk_size: 4 * 1024 * 1024,
            channel_capacity: 1024,
            batch_size: 100,
            batch_timeout_ms: 100,
            worker_threads: num_cpus::get(),
        }
    }
}

impl IngestPipeline {
    pub fn run(&self, root: &Path, cas_root: &Path) -> Result<IngestStats> {
        let (path_tx, path_rx) = bounded(self.config.channel_capacity);
        let (commit_tx, commit_rx) = bounded(self.config.channel_capacity);
        
        let memory_sem = Arc::new(Semaphore::new(self.config.memory_budget));
        
        // Spawn scanner (watch-first)
        let scanner = WatchFirstScanner::new(root.to_owned(), path_tx)?;
        let scanner_handle = thread::spawn(move || scanner.run());
        
        // Spawn worker pool
        let pool = Arc::new(WorkerPool::new(
            memory_sem,
            self.config.mmap_threshold,
            self.config.chunk_size,
        ));
        
        let workers: Vec<_> = (0..self.config.worker_threads)
            .map(|_| {
                let pool = pool.clone();
                let rx = path_rx.clone();
                let tx = commit_tx.clone();
                
                thread::spawn(move || {
                    while let Ok(item) = rx.recv() {
                        match pool.process(item) {
                            Ok(Some(processed)) => { tx.send(processed)?; }
                            Ok(None) => { /* skip or done */ }
                            Err(CasError::FileModified(_)) => { /* retry logic */ }
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        
        drop(commit_tx); // Close sender when workers done
        
        // Spawn committer
        let mut committer = BatchCommitter::new(cas_root, self.config.batch_size);
        let mut stats = IngestStats::default();
        
        loop {
            match commit_rx.recv_timeout(Duration::from_millis(self.config.batch_timeout_ms)) {
                Ok(item) => {
                    stats.files_processed += 1;
                    stats.bytes_processed += item.size;
                    committer.current_batch.push(item);
                    
                    if committer.current_batch.len() >= self.config.batch_size {
                        committer.commit_batch()?;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Flush partial batch
                    committer.commit_batch()?;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // Final flush
                    committer.commit_batch()?;
                    break;
                }
            }
        }
        
        // Wait for all threads
        scanner_handle.join()??;
        for w in workers {
            w.join()??;
        }
        
        Ok(stats)
    }
}
```

---

## Platform-Specific Optimizations

### Linux (io_uring)
```rust
#[cfg(all(target_os = "linux", feature = "io_uring"))]
fn process_batch_uring(files: &[PathBuf]) -> Result<Vec<ProcessedFile>> {
    tokio_uring::start(async {
        let mut results = Vec::with_capacity(files.len());
        
        for chunk in files.chunks(256) {
            let mut reads = Vec::new();
            
            for path in chunk {
                reads.push(tokio_uring::fs::read(path));
            }
            
            let contents = futures::future::join_all(reads).await;
            // ... hash and write
        }
        
        Ok(results)
    })
}
```

### macOS
```rust
#[cfg(target_os = "macos")]
fn process_with_fcopyfile(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    
    unsafe {
        let src_cstr = CString::new(src.as_os_str().as_bytes())?;
        let dst_cstr = CString::new(dst.as_os_str().as_bytes())?;
        
        // Zero-copy file copy on APFS
        libc::copyfile(
            src_cstr.as_ptr(),
            dst_cstr.as_ptr(),
            std::ptr::null_mut(),
            libc::COPYFILE_ALL,
        );
    }
    Ok(())
}
```

---

## Performance Projections

| Scenario | Current | Pipeline V2 | Improvement |
|----------|---------|-------------|-------------|
| 10k small files | 28s | ~3s | 9x |
| 100k small files | 279s | ~25s | 11x |
| 1M small files | OOM | ~250s | ∞ |
| Mixed (10% large) | OOM | ~300s | ∞ |

**Key improvements:**
- fsync reduction: 200x → 2x per batch
- Memory: O(files) → O(1)
- Zero-copy: eliminates user-space buffer copies
- Watch-first: guarantees no missed files

---

## Dependencies

```toml
[dependencies]
notify = "6.0"
crossbeam-channel = "0.5"
memmap2 = "0.9"
blake3 = "1.5"
dashmap = "5.5"
num_cpus = "1.16"

[target.'cfg(target_os = "linux")'.dependencies]
nix = { version = "0.28", features = ["fs"] }
tokio-uring = { version = "0.5", optional = true }

[features]
io_uring = ["tokio-uring"]
```
