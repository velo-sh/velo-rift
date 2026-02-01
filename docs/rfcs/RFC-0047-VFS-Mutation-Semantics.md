# RFC-0047: VFS Mutation Semantics

## Status: Draft

---

## Abstract

Velo Rift provides a transparent virtual filesystem for compilation acceleration. This RFC defines the mutation semantics: how `write`, `unlink`, `rename`, `mkdir`, `rmdir` behave within the VFS.

---

## Core Principles

1. **CAS is immutable** - Content-addressable blobs are never modified or deleted by syscalls
2. **Manifest is the view** - User sees paths defined by Manifest entries
3. **Transparent to compilers** - All operations succeed; VFS is invisible

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│  User/Compiler sees:  /vrift/project/...        │
│  (Pure Virtual View)                            │
└─────────────────────────────────────────────────┘
                       ↓
┌─────────────────────────────────────────────────┐
│  Manifest (per-project, mutable)                │
│  /src/main.rs → blake3:abc123                   │
│  /src/lib.rs  → blake3:def456                   │
│  /target/     → (directory entry)               │
└─────────────────────────────────────────────────┘
                       ↓
┌─────────────────────────────────────────────────┐
│  CAS (TheSource, global, immutable)             │
│  abc123 → [content bytes, refcount=5]           │
│  def456 → [content bytes, refcount=2]           │
└─────────────────────────────────────────────────┘
```

---

## Syscall Behavior

| Syscall | Action | Manifest | CAS |
|---------|--------|----------|-----|
| `open(O_RDONLY)` | Return CAS content | Read entry | No change |
| `open(O_WRONLY)` | Create temp, track FD | - | - |
| `write` | Write to temp | - | - |
| `close` (dirty FD) | Hash content → insert CAS → update Manifest | Update entry | Insert new blob |
| `unlink` | Remove entry | Delete entry | No change |
| `rename(a, b)` | Move entry | a→b path change | No change |
| `mkdir` | Create dir entry | Add dir entry | No change |
| `rmdir` | Remove empty dir | Delete entry | No change |

---

## Write Path (CoW)

```
1. open("/vrift/project/file.txt", O_WRONLY)
   → Create temp file in /tmp/vrift-cow-xxx
   → Track FD → temp path mapping

2. write(fd, data)
   → Write to temp file

3. close(fd)
   → Hash temp content → blake3:xyz789
   → Insert blob into CAS (if not exists)
   → Update Manifest: /file.txt → xyz789
   → Delete temp file
```

---

## Delete Path

```
unlink("/vrift/project/old.rs")
   → Lookup Manifest: /old.rs → abc123
   → Remove entry from Manifest
   → Return 0 (success)
   → CAS blob abc123 remains (may be shared)
```

---

## Rename Path

```
rename("/vrift/project/a.rs", "/vrift/project/b.rs")
   → Lookup Manifest: /a.rs → abc123
   → Remove entry /a.rs
   → Add entry /b.rs → abc123
   → Return 0 (success)
```

---

## Implementation Requirements

1. **Shim must modify Manifest** - Not passthrough to real FS
2. **IPC to Daemon** - Manifest updates via daemon for atomicity
3. **FD Tracking** - Map open FDs to temp files for CoW

---

## Current vs Target State

| Syscall | Current | Target |
|---------|---------|--------|
| `unlink` | ❌ EROFS | ✅ Remove Manifest entry |
| `rename` | ❌ EROFS | ✅ Update Manifest path |
| `mkdir` | ⏳ Passthrough | ✅ Add Manifest dir entry |
| `rmdir` | ❌ EROFS | ✅ Remove Manifest dir entry |
| `write` | ⏳ Partial CoW | ✅ Full CoW with CAS insert |
