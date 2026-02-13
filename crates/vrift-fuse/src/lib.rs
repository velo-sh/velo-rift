//! # velo-fuse
//!
//! FUSE filesystem implementation for Velo Rift.
//!
//! Maps the Velo Manifest and CAS to a FUSE filesystem.
//! - Inodes are assigned sequentially based on manifest entries.
//! - Read operations fetch from CAS.
//! - Metadata comes from Manifest.

#[cfg(all(feature = "fuse", target_os = "linux"))]
mod imp {
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    use fuser::{
        FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
    };
    use libc::{c_int, ENOENT};
    use vrift_cas::Blake3Hash;
    use vrift_cas::CasStore;
    use vrift_manifest::{LmdbManifest, VnodeEntry};

    const TTL: Duration = Duration::from_secs(60);
    const BLOCK_SIZE: u64 = 4096;

    struct InodeEntry {
        content_hash: Blake3Hash,
        attr: FileAttr,
        children: Vec<(String, u64)>, // Name -> Inode
    }

    pub struct VeloFs {
        cas: CasStore,
        inodes: HashMap<u64, InodeEntry>,
        path_to_inode: HashMap<String, u64>,
    }

    impl VeloFs {
        pub fn new(manifest: &LmdbManifest, cas: CasStore) -> Self {
            let mut fs = Self {
                cas,
                inodes: HashMap::new(),
                path_to_inode: HashMap::new(),
            };
            fs.init_from_manifest(manifest);
            fs
        }

        /// Mount the filesystem at the given path (Ref: <https://docs.rs/fuser>)
        pub fn mount(self, mountpoint: &Path) -> anyhow::Result<()> {
            // Performance Optimization: TTL=60s (Implies metadata caching).
            // auto_cache / kernel_cache causing issues with current fuser/libfuse version in CI.
            // TTL=60s provides significant getattr reduction.

            let opts = vec![
                fuser::MountOption::RO,
                fuser::MountOption::FSName("vrift".to_string()),
            ];

            fuser::mount2(self, mountpoint, &opts)?;
            Ok(())
        }

        fn init_from_manifest(&mut self, manifest: &LmdbManifest) {
            // 1. Assign inodes to all paths
            let mut next_inode = 2; // 1 is root

            // Ensure root exists
            self.inodes.insert(
                1,
                InodeEntry {
                    content_hash: [0u8; 32],
                    attr: Self::default_dir_attr(1),
                    children: Vec::new(),
                },
            );
            self.path_to_inode.insert("/".to_string(), 1);

            // Sort paths to process parents before children (ensures directory structure)
            let mut entries = manifest.iter().unwrap_or_default();
            entries.sort_by(|a, b| a.0.cmp(&b.0));

            for (path, entry) in entries {
                if path == "/" {
                    continue;
                } // Already handled

                let inode = next_inode;
                next_inode += 1;

                self.path_to_inode.insert(path.clone(), inode);

                let attr = Self::vnode_to_attr(inode, &entry.vnode);

                self.inodes.insert(
                    inode,
                    InodeEntry {
                        content_hash: entry.vnode.content_hash,
                        attr,
                        children: Vec::new(),
                    },
                );

                // Add to parent's children
                let p = Path::new(&path);
                if let Some(parent) = p.parent() {
                    let parent_str = if parent == Path::new("") {
                        "/"
                    } else {
                        parent.to_str().unwrap()
                    };

                    // Handle implicit directories
                    let parent_inode = if let Some(&inode) = self.path_to_inode.get(parent_str) {
                        inode
                    } else {
                        self.create_implicit_dirs(parent_str, &mut next_inode)
                    };

                    let name = p.file_name().unwrap().to_str().unwrap().to_string();
                    if let Some(parent_entry) = self.inodes.get_mut(&parent_inode) {
                        parent_entry.children.push((name, inode));
                    }
                }
            }
        }

        fn create_implicit_dirs(&mut self, path: &str, next_inode: &mut u64) -> u64 {
            if let Some(&inode) = self.path_to_inode.get(path) {
                return inode;
            }

            // Recursively ensure parent exists
            let p = Path::new(path);
            let parent_inode = if let Some(parent) = p.parent() {
                let parent_str = if parent == Path::new("") {
                    "/"
                } else {
                    parent.to_str().unwrap()
                };
                self.create_implicit_dirs(parent_str, next_inode)
            } else {
                1 // Fallback to root
            };

            let inode = *next_inode;
            *next_inode += 1;

            self.path_to_inode.insert(path.to_string(), inode);

            // Create directory entry
            let attr = Self::default_dir_attr(inode);
            self.inodes.insert(
                inode,
                InodeEntry {
                    content_hash: [0u8; 32],
                    attr,
                    children: Vec::new(),
                },
            );

            // Link to parent
            if let Some(parent_entry) = self.inodes.get_mut(&parent_inode) {
                let name = p.file_name().unwrap().to_str().unwrap().to_string();
                parent_entry.children.push((name, inode));
            }

            inode
        }

        fn default_dir_attr(inode: u64) -> FileAttr {
            FileAttr {
                ino: inode,
                size: 0,
                blocks: 0,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
                crtime: UNIX_EPOCH,
                kind: FileType::Directory,
                perm: 0o755,
                nlink: 2,
                uid: 0,
                gid: 0,
                rdev: 0,
                flags: 0,
                blksize: BLOCK_SIZE as u32,
            }
        }

        fn vnode_to_attr(inode: u64, vnode: &VnodeEntry) -> FileAttr {
            FileAttr {
                ino: inode,
                size: vnode.size,
                blocks: vnode.size.div_ceil(BLOCK_SIZE),
                atime: UNIX_EPOCH + Duration::from_secs(vnode.mtime),
                mtime: UNIX_EPOCH + Duration::from_secs(vnode.mtime),
                ctime: UNIX_EPOCH + Duration::from_secs(vnode.mtime),
                crtime: UNIX_EPOCH + Duration::from_secs(vnode.mtime),
                kind: if vnode.is_dir() {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                },
                perm: vnode.mode as u16,
                nlink: if vnode.is_dir() { 2 } else { 1 },
                uid: 0,
                gid: 0,
                rdev: 0,
                flags: 0,
                blksize: BLOCK_SIZE as u32,
            }
        }
    }

    impl Filesystem for VeloFs {
        fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let name_str = match name.to_str() {
                Some(s) => s,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };

            if let Some(parent_entry) = self.inodes.get(&parent) {
                for (child_name, child_inode) in &parent_entry.children {
                    if child_name == name_str {
                        if let Some(child_entry) = self.inodes.get(child_inode) {
                            reply.entry(&TTL, &child_entry.attr, 0);
                            return;
                        }
                    }
                }
            }
            reply.error(ENOENT);
        }

        fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
            match self.inodes.get(&ino) {
                Some(entry) => reply.attr(&TTL, &entry.attr),
                None => reply.error(ENOENT),
            }
        }

        fn read(
            &mut self,
            _req: &Request,
            ino: u64,
            _fh: u64,
            offset: i64,
            size: u32,
            _flags: c_int,
            _lock_owner: Option<u64>,
            reply: ReplyData,
        ) {
            let entry = match self.inodes.get(&ino) {
                Some(e) => e,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };

            match self.cas.get(&entry.content_hash) {
                Ok(data) => {
                    let offset = offset as usize;
                    let size = size as usize;
                    if offset >= data.len() {
                        reply.data(&[]);
                    } else {
                        let end = (offset + size).min(data.len());
                        reply.data(&data[offset..end]);
                    }
                }
                Err(e) => {
                    eprintln!("CAS Read Error for inode {}: {}", ino, e);
                    reply.error(libc::EIO);
                }
            }
        }

        fn readdir(
            &mut self,
            _req: &Request,
            ino: u64,
            _fh: u64,
            offset: i64,
            mut reply: ReplyDirectory,
        ) {
            let entry = match self.inodes.get(&ino) {
                Some(e) => e,
                None => {
                    reply.error(ENOENT);
                    return;
                }
            };

            if offset == 0 {
                if reply.add(ino, 0, FileType::Directory, ".") {
                    return;
                }
                if reply.add(1, 1, FileType::Directory, "..") {
                    // Parent hardcoded to 1 for simplicity for now
                    return;
                }
            }

            // Children
            // Skip 'offset' entries (simplified logic: FUSE readdir offset usually starts at 1/2/3...)
            // Real implementations need robust offset handling.
            // Here we just use index in child array + 2 (for . and ..)
            let skip = if offset > 1 { (offset - 2) as usize } else { 0 };

            for (i, (name, child_ino)) in entry.children.iter().enumerate().skip(skip) {
                let child_type = self
                    .inodes
                    .get(child_ino)
                    .map(|e| e.attr.kind)
                    .unwrap_or(FileType::RegularFile);
                // offset for next entry is i + 3 (1-based index + 2 for dots)
                if reply.add(*child_ino, (i + 3) as i64, child_type, name) {
                    break;
                }
            }
            reply.ok();
        }
    }
}

#[cfg(not(all(feature = "fuse", target_os = "linux")))]
mod imp {
    use vrift_cas::CasStore;
    use vrift_manifest::LmdbManifest;

    /// Dummy FUSE filesystem for non-Linux or non-feature builds
    pub struct VeloFs;

    impl VeloFs {
        pub fn new(_manifest: &LmdbManifest, _cas: CasStore) -> Self {
            #[cfg(not(target_os = "linux"))]
            println!(
                "⚠️  FUSE support is only available on Linux (current: {}).",
                std::env::consts::OS
            );
            #[cfg(all(target_os = "linux", not(feature = "fuse")))]
            println!("⚠️  VeloFs is disabled. Compile with --features fuse to enable.");
            Self
        }

        pub fn mount(self, _mountpoint: &std::path::Path) -> anyhow::Result<()> {
            anyhow::bail!("FUSE not supported on this platform");
        }
    }
}

pub use imp::VeloFs;
