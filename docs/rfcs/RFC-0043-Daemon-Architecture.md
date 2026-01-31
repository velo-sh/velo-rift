# RFC-0043: Daemon Architecture

- **Status**: Draft
- **Created**: 2026-01-31
- **Author**: Velo Rift Team

## Summary

Implement a system daemon (`vrift-daemon`) that runs under a dedicated user account (`vrift`) to provide:
1. Secure CAS ownership isolation
2. Multi-user CAS sharing
3. Live Ingest via IPC (RFC-0039)
4. Zero-privilege client operations

## Motivation

RFC-0039 requires immutable Tier-1 assets protected via `chattr +i`, which requires root or `CAP_LINUX_IMMUTABLE`. This is unacceptable for developer tools.

**Solution**: Run daemon under dedicated `vrift` user, which owns all CAS blobs. Regular users interact via IPC, cannot modify CAS directly.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    User Space                            │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐  │
│  │  Project A  │    │  Project B  │    │  Project C  │  │
│  │  (user: dev)│    │  (user: dev)│    │(user: alice)│  │
│  └──────┬──────┘    └──────┬──────┘    └──────┬──────┘  │
│         │ symlink          │ symlink          │ symlink │
└─────────┼──────────────────┼──────────────────┼─────────┘
          │                  │                  │
          ▼                  ▼                  ▼
┌──────────────────────────────────────────────────────────┐
│                vrift-daemon (user: vrift)                │
│  ┌────────────────────────────────────────────────────┐  │
│  │  /var/lib/vrift/the_source/                        │  │
│  │  owner: vrift:vrift                                │  │
│  │  mode:  drwxr-xr-x (755)                           │  │
│  │                                                    │  │
│  │  blake3/ab/cd/hash_size.bin                        │  │
│  │  owner: vrift:vrift                                │  │
│  │  mode:  -r--r--r-- (444)                           │  │
│  └────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

## IPC Protocol

### Transport

- **Linux**: Unix Domain Socket at `/run/vrift/daemon.sock`
- **macOS**: Unix Domain Socket at `~/Library/Application Support/vrift/daemon.sock`

### Messages (JSON-RPC 2.0)

```json
// Request: Ingest file
{
  "jsonrpc": "2.0",
  "method": "ingest",
  "params": {
    "path": "/home/dev/project/node_modules",
    "tier": "tier1",
    "mode": "solid"
  },
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "files": 12345,
    "bytes": 104857600,
    "unique": 8765,
    "manifest_path": "/home/dev/project/.vrift/manifest.lmdb"
  },
  "id": 1
}
```

### Methods

| Method | Description |
|--------|-------------|
| `ingest` | Ingest directory into CAS |
| `link` | Create symlink projection |
| `status` | Get daemon/CAS status |
| `gc` | Run garbage collection |

## Security Model

### User Isolation

| Component | Owner | Permissions |
|-----------|-------|-------------|
| `/var/lib/vrift/` | `vrift:vrift` | `drwxr-xr-x (755)` |
| CAS blobs | `vrift:vrift` | `-r--r--r-- (444)` |
| Socket | `vrift:vrift` | `srw-rw-rw- (666)` |

### Benefits

1. **No root required**: Daemon runs as unprivileged `vrift` user
2. **CAS integrity**: Users cannot modify blobs (wrong owner)
3. **Deduplication**: Shared CAS across all users on system
4. **Audit trail**: All writes go through daemon

## Installation

### Package Scripts (deb/rpm)

```bash
# Post-install script
getent group vrift >/dev/null || groupadd -r vrift
getent passwd vrift >/dev/null || \
    useradd -r -g vrift -d /var/lib/vrift -s /sbin/nologin \
    -c "Velo Rift Daemon" vrift

mkdir -p /var/lib/vrift/the_source
mkdir -p /run/vrift
chown -R vrift:vrift /var/lib/vrift /run/vrift
chmod 755 /var/lib/vrift
```

### systemd Service

```ini
[Unit]
Description=Velo Rift Daemon
After=network.target

[Service]
Type=notify
User=vrift
Group=vrift
RuntimeDirectory=vrift
ExecStart=/usr/bin/vrift-daemon
Restart=on-failure
RestartSec=5

# Security hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/var/lib/vrift

[Install]
WantedBy=multi-user.target
```

## Client-Daemon Communication Flow

```
1. User runs: vrift ingest ./node_modules

2. CLI detects daemon mode (socket exists)

3. CLI sends IPC request to daemon

4. Daemon (as vrift user):
   - Reads files from user's directory
   - Computes hashes
   - Stores blobs in /var/lib/vrift/the_source/
   - Sets permissions (444)
   - Returns manifest info

5. CLI creates symlinks in user's project
   (symlinks owned by user, pointing to vrift-owned CAS)
```

## Fallback Mode

When daemon is not available (dev/testing):

```
vrift ingest ./node_modules  # No daemon
  → Falls back to direct mode
  → CAS at ~/.vrift/the_source/ (user-owned)
  → chmod 444 only (no user isolation)
```

## Implementation Phases

| Phase | Scope | Priority |
|-------|-------|----------|
| P1 | IPC protocol, basic daemon | High |
| P2 | systemd integration, packaging | High |
| P3 | macOS launchd support | Medium |
| P4 | Multi-user quotas, metrics | Low |

## References

- RFC-0039: Transparent Virtual Projection
- Nix Daemon: https://nixos.org/manual/nix/stable/installation/multi-user
- systemd.exec: https://www.freedesktop.org/software/systemd/man/systemd.exec.html
