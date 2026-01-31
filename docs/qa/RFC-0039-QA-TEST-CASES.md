# RFC-0039 QA Test Cases

## 1. Iron Law Enforcement (CAS Immutability)

| ID | Description | Mode | Tier | Expected Result |
|----|-------------|------|------|-----------------|
| TC-IL-01 | Attempt to delete a CAS blob as root | N/A | Tier-1 | `EPERM` (macOS uchg / Linux +i) |
| TC-IL-02 | Attempt to write to a CAS blob via hardlink | Solid | Tier-2 | `EACCES` (chmod 444) |
| TC-IL-03 | Verify execution bits are stripped from ingested blobs | N/A | All | `stat` shows No Execute for any user |

## 2. Zero-Copy Integrity

| ID | Description | Mode | Tier | Expected Result |
|----|-------------|------|------|-----------------|
| TC-ZC-01 | Concurrent ingest of the same file (flock) | Solid | Tier-1 | One process hydrates, others block or skip. Final hash matches. |
| TC-ZC-02 | Ingest a 1GB file with zero-copy | Solid | Tier-1 | Near O(1) duration; No increase in disk usage (shared block) |
| TC-ZC-03 | Phantom Mode: Move large directory to CAS | Phantom | N/A | O(1) `rename` behavior; Source paths are removed |

## 3. VFS Shim: Break-Before-Write (BBW)

| ID | Description | Mode | Tier | Expected Result |
|----|-------------|------|------|-----------------|
| TC-BBW-01 | Open Tier-2 file with `O_WRONLY` | Solid | Tier-2 | Shim creates private temp copy. Original CAS blob UNCHANGED. |
| TC-BBW-02 | `write()` to Tier-2 file | Solid | Tier-2 | Writes redirected to temp file. |
| TC-BBW-03 | `close()` of modified Tier-2 file | Solid | Tier-2 | Shim computes new hash and stores in CAS. |
| TC-BBW-04 | Optimization: `O_TRUNC` on open | Solid | Tier-2 | Shim skips copying old content to temp file. |

## 4. Manifest Synchronization

| ID | Description | Mode | Tier | Expected Result |
|----|-------------|------|------|-----------------|
| TC-MS-01 | Reload session after BBW re-ingest | Solid | Tier-2 | Manifest shows new hash for the modified path. |
| TC-MS-02 | Delta Layer whiteout (Phantom Mode) | Phantom | N/A | `rm <physical_path>` reflects correctly in VFS view. |

## 5. ABI Continuity (Dimensional Ingest)

| ID | Description | Mode | Tier | Expected Result |
|----|-------------|------|------|-----------------|
| TC-ABI-01 | Ingest same path with different `VRIFT_ABI_CONTEXT` | Solid | Tier-1 | CAS contains two distinct blobs. VFS redirects to correct one. |
