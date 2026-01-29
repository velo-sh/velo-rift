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
    use velo_cas::CasStore;
    use velo_manifest::{Manifest, VnodeEntry};

    const TTL: Duration = Duration::from_secs(1);
    const BLOCK_SIZE: u64 = 4096;

    struct InodeEntry {
        path_hash: velo_manifest::PathHash,
        attr: FileAttr,
        children: Vec<(String, u64)>, // Name -> Inode
    }

    pub struct VeloFs {
        cas: CasStore,
        inodes: HashMap<u64, InodeEntry>,
        path_to_inode: HashMap<String, u64>,
    }

    impl VeloFs {
        pub fn new(manifest: &Manifest, cas: CasStore) -> Self {
            let mut fs = Self {
                cas,
                inodes: HashMap::new(),
                path_to_inode: HashMap::new(),
            };
            fs.init_from_manifest(manifest);
            fs
        }

        fn init_from_manifest(&mut self, manifest: &Manifest) {
            // 1. Assign inodes to all paths
            let mut next_inode = 2; // 1 is root

            // Ensure root exists
            self.inodes.insert(
                1,
                InodeEntry {
                    path_hash: [0; 32], // Dummy
                    attr: Self::default_dir_attr(1),
                    children: Vec::new(),
                },
            );
            self.path_to_inode.insert("/".to_string(), 1);

            // Sort paths to process parents before children (ensures directory structure)
            let mut paths: Vec<&str> = manifest.paths().collect();
            paths.sort();

            for path in paths {
                if path == "/" {
                    continue;
                } // Already handled

                let inode = next_inode;
                next_inode += 1;

                self.path_to_inode.insert(path.to_string(), inode);

                let entry = manifest.get(path).unwrap();
                let attr = Self::vnode_to_attr(inode, entry);

                self.inodes.insert(
                    inode,
                    InodeEntry {
                        path_hash: entry.content_hash,
                        attr,
                        children: Vec::new(),
                    },
                );

                // Add to parent's children
                let p = Path::new(path);
                if let Some(parent) = p.parent() {
                    let parent_str = if parent == Path::new("") {
                        "/"
                    } else {
                        parent.to_str().unwrap()
                    };
                    // Ensure normalized path (e.g., if parent is empty, it's root)
                    // Or better: ensure we find the parent inode
                    if let Some(parent_inode) = self.path_to_inode.get(parent_str) {
                        let name = p.file_name().unwrap().to_str().unwrap().to_string();
                        if let Some(parent_entry) = self.inodes.get_mut(parent_inode) {
                            parent_entry.children.push((name, inode));
                        }
                    } else {
                        // Parent might be missing from manifest if explicit entries omitted?
                        // For MVP assume valid manifest.
                        // Or auto-create implicit directories?
                        // Let's assume manifest is complete.
                    }
                }
            }
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

            match self.cas.get(&entry.path_hash) {
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
                Err(_) => reply.error(libc::EIO),
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
    use velo_cas::CasStore;
    use velo_manifest::Manifest;

    /// Dummy FUSE filesystem for non-Linux or non-feature builds
    pub struct VeloFs;

    impl VeloFs {
        pub fn new(_manifest: &Manifest, _cas: CasStore) -> Self {
            #[cfg(not(target_os = "linux"))]
            println!(
                "⚠️  FUSE support is only available on Linux (current: {}).",
                std::env::consts::OS
            );
            #[cfg(all(target_os = "linux", not(feature = "fuse")))]
            println!("⚠️  VeloFs is disabled. Compile with --features fuse to enable.");
            Self
        }
    }
}

pub use imp::VeloFs;
