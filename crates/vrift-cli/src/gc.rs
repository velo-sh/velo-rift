//! # Garbage Collection (RFC-0041)
//!
//! Multi-manifest garbage collection with registry integration and Bloom-assisted daemon sweep.
//! Supports time-based cleanup via `--unused-for` for removing stale CAS blobs directly.

use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use vrift_cas::CasStore;
use vrift_manifest::LmdbManifest;

use crate::registry::ManifestRegistry;

#[derive(Args, Debug)]
pub struct GcArgs {
    /// Path to a single Manifest (LMDB directory)
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Actually delete blobs (default is dry-run)
    #[arg(long)]
    delete: bool,

    /// Remove stale manifest entries before GC
    #[arg(long)]
    prune_stale: bool,

    /// Skip confirmation prompt (for scripts and CI)
    #[arg(long, short = 'y', default_value = "false")]
    yes: bool,

    /// Delete CAS blobs not used within this duration (e.g. "24h", "7d", "1h").
    /// Uses max(atime, mtime) to determine last-used time, so recently-read
    /// old blobs are preserved. Direct file cleanup, no daemon required.
    /// Handles macOS immutable flags (uchg) automatically.
    ///
    /// ⚠️  This is a DESTRUCTIVE operation:
    ///   - CAS blobs unused for the specified duration are permanently deleted
    ///   - Manifest entries referencing deleted blobs are removed
    ///   - Affected projects will need re-ingest (`vrift ingest`) to restore
    ///   - VDir entries pointing to deleted blobs will miss (graceful fallback)
    #[arg(long, value_parser = parse_duration)]
    unused_for: Option<Duration>,
}

/// Parse a human-readable duration string like "24h", "7d", "30m", "1h30m"
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let mut total_secs: u64 = 0;
    let mut num_buf = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            num_buf.push(c);
        } else {
            let n: u64 = num_buf
                .parse()
                .map_err(|_| format!("Invalid number in duration: {}", s))?;
            num_buf.clear();
            match c {
                'd' => total_secs += n * 86400,
                'h' => total_secs += n * 3600,
                'm' => total_secs += n * 60,
                's' => total_secs += n,
                _ => return Err(format!("Unknown duration unit '{}' in '{}'", c, s)),
            }
        }
    }
    // Handle bare number (default to hours for convenience)
    if !num_buf.is_empty() {
        let n: u64 = num_buf
            .parse()
            .map_err(|_| format!("Invalid number in duration: {}", s))?;
        total_secs += n * 3600; // Default unit is hours
    }

    if total_secs == 0 {
        return Err("Duration must be greater than zero".to_string());
    }
    Ok(Duration::from_secs(total_secs))
}

pub async fn run(cas_root: &Path, args: GcArgs) -> Result<()> {
    // If --unused-for is specified, use the time-based direct cleanup path
    if let Some(unused_for) = args.unused_for {
        return run_unused_for_gc(cas_root, unused_for, args.delete, args.yes).await;
    }

    println!();
    println!("🗑️  VRift Garbage Collection");
    println!("   CAS:     {}", cas_root.display());

    // Acquire exclusive lock
    let _lock = ManifestRegistry::acquire_lock().context("Failed to acquire registry lock")?;

    // Load or create registry
    let mut registry = ManifestRegistry::load_or_create()?;

    // Verify all manifests to detect stale ones
    let (active_count, stale_count) = registry.verify_all();

    // Collect all referenced blob hashes
    let keep_set: HashSet<_> = if let Some(ref manifest_path) = args.manifest {
        println!();
        println!("  [Direct Mode] Using single manifest: {:?}", manifest_path);
        // RFC-0039: Manifest is an LMDB directory
        let manifest = LmdbManifest::open(manifest_path).context("Failed to open LMDB manifest")?;
        let entries = manifest.iter().context("Failed to iterate manifest")?;
        entries
            .into_iter()
            .map(|(_, m_entry)| m_entry.vnode.content_hash)
            .collect()
    } else {
        println!();
        println!("  Registry Status:");
        println!(
            "    📁 Registered manifests: {} ({} active, {} stale)",
            registry.manifests.len(),
            active_count,
            stale_count
        );

        if args.prune_stale && stale_count > 0 {
            let pruned = registry.prune_stale();
            registry.save()?;
            println!("    🗑️  Pruned {} stale manifest entries", pruned);
        }

        registry
            .get_all_blob_hashes()
            .context("Failed to collect blob hashes from manifests")?
    };

    println!();
    println!(
        "  ✅ Referenced blobs: {}",
        format_number(keep_set.len() as u64)
    );

    // Build Bloom Filter from keep_set
    use vrift_ipc::{BloomFilter, BLOOM_SIZE};
    let mut bloom = BloomFilter::new(BLOOM_SIZE);
    for hash in &keep_set {
        bloom.add(&CasStore::hash_to_hex(hash));
    }

    if args.delete {
        if !args.yes {
            println!();
            print!("  ⚠️  Proceed with Bloom-assisted GC sweep on daemon? [y/N] ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("  Cancelled.");
                return Ok(());
            }
        }

        let gc_start = Instant::now();
        println!("  🧼 Triggering CAS sweep via daemon...");

        // Connect to daemon and send sweep request
        use vrift_ipc::{VeloRequest, VeloResponse};
        let project_root = std::env::current_dir().context("Failed to get current directory")?;
        let conn = crate::daemon::connect_to_daemon(&project_root)
            .await
            .context("Daemon not running or unreachable")?;
        let mut stream = conn.stream;
        crate::daemon::send_request(
            &mut stream,
            VeloRequest::CasSweep {
                bloom_filter: bloom.bits.clone(),
            },
        )
        .await?;

        match crate::daemon::read_response(&mut stream).await? {
            VeloResponse::CasSweepAck {
                deleted_count,
                reclaimed_bytes,
            } => {
                let gc_elapsed = gc_start.elapsed().as_secs_f64();
                println!();
                println!("╔════════════════════════════════════════╗");
                println!("║  ✅ GC Complete in {:.2}s              ║", gc_elapsed);
                println!("╚════════════════════════════════════════╝");
                println!();
                println!(
                    "   🗑️  {} orphaned blobs deleted",
                    format_number(deleted_count as u64)
                );
                println!("   💾 {} reclaimed", format_bytes(reclaimed_bytes));
            }
            VeloResponse::Error(e) => return Err(anyhow::anyhow!("Sweep failed: {}", e)),
            _ => return Err(anyhow::anyhow!("Unexpected response from daemon")),
        }
    } else {
        println!("\n  📋 Dry Run: Scanning CAS for orphaned blobs...");
        let cas = CasStore::new(cas_root)?;
        let mut orphan_count = 0u64;
        let mut orphan_bytes = 0u64;

        if let Ok(iter) = cas.iter() {
            for hash in iter.flatten() {
                if !keep_set.contains(&hash) {
                    orphan_count += 1;
                    if let Some(p) = cas.blob_path_for_hash(&hash) {
                        if let Ok(meta) = std::fs::metadata(p) {
                            orphan_bytes += meta.len();
                        }
                    }
                }
            }
        }

        if orphan_count > 0 {
            println!(
                "   ⚠️  {} orphans found ({})",
                format_number(orphan_count),
                format_bytes(orphan_bytes)
            );
        } else {
            println!("   ✅ No orphans found.");
        }

        println!();
        println!("     👉 Run with --delete to trigger daemon sweep.");
    }

    // Save registry
    registry.save()?;
    println!();
    Ok(())
}

/// Time-based GC: delete CAS blob files not used within `unused_for` duration.
///
/// Safe cleanup flow:
///   1. Scan CAS for stale files by max(atime, mtime)
///   2. Extract blob hashes from stale filenames
///   3. Remove matching manifest entries (prevents dangling refs)
///   4. Commit manifest changes
///   5. Delete CAS files (handles macOS immutable flags)
///   6. Clean empty directories
async fn run_unused_for_gc(
    cas_root: &Path,
    max_age: Duration,
    delete: bool,
    yes: bool,
) -> Result<()> {
    let hours = max_age.as_secs() / 3600;
    let mins = (max_age.as_secs() % 3600) / 60;
    let age_str = if mins > 0 {
        format!("{}h{}m", hours, mins)
    } else {
        format!("{}h", hours)
    };

    println!();
    println!("🗑️  VRift Time-Based Garbage Collection");
    println!("   CAS:      {}", cas_root.display());
    println!("   Max age:  {} (last-used = max(atime, mtime))", age_str);
    println!();

    let cutoff = SystemTime::now() - max_age;
    let blake3_dir = cas_root.join("blake3");

    if !blake3_dir.exists() {
        println!("  ✅ CAS directory is empty, nothing to clean.");
        return Ok(());
    }

    println!("  📋 Scanning CAS for blobs not used in {}...", age_str);

    let gc_start = Instant::now();
    let mut stale_count = 0u64;
    let mut stale_bytes = 0u64;
    let mut kept_count = 0u64;
    let mut kept_bytes = 0u64;
    let mut error_count = 0u64;
    let mut stale_files: Vec<PathBuf> = Vec::new();
    // Collect stale blob hashes for manifest cleanup
    let mut stale_hashes: HashSet<[u8; 32]> = HashSet::new();

    // Walk the blake3/XX/YY/ directory structure
    let l1_entries = std::fs::read_dir(&blake3_dir).context("Failed to read CAS blake3 dir")?;
    for l1 in l1_entries.flatten() {
        if !l1.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let l2_entries = match std::fs::read_dir(l1.path()) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for l2 in l2_entries.flatten() {
            if !l2.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let l3_entries = match std::fs::read_dir(l2.path()) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for l3 in l3_entries.flatten() {
                let path = l3.path();
                let meta = match std::fs::metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !meta.is_file() {
                    continue;
                }
                // Use max(atime, mtime) as "last used" time.
                // mtime = when blob was stored; atime = when last read.
                // This preserves recently-accessed old blobs.
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let atime = meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH);
                let last_used = std::cmp::max(atime, mtime);
                if last_used < cutoff {
                    stale_count += 1;
                    stale_bytes += meta.len();
                    if delete {
                        // Extract blake3 hash from filename: "HASH_SIZE.bin" -> HASH
                        if let Some(hash) = extract_hash_from_cas_filename(&path) {
                            stale_hashes.insert(hash);
                        }
                        stale_files.push(path);
                    }
                } else {
                    kept_count += 1;
                    kept_bytes += meta.len();
                }
            }
        }
    }

    let scan_elapsed = gc_start.elapsed().as_secs_f64();
    println!("  ✅ Scan complete in {:.1}s", scan_elapsed);
    println!();
    println!("  📊 Results (by last-used = max(atime, mtime)):");
    println!(
        "     Stale (unused > {}):  {} files ({})",
        age_str,
        format_number(stale_count),
        format_bytes(stale_bytes)
    );
    println!(
        "     Fresh (used < {}):    {} files ({})",
        age_str,
        format_number(kept_count),
        format_bytes(kept_bytes)
    );

    if stale_count == 0 {
        println!();
        println!("  ✅ Nothing to clean.");
        return Ok(());
    }

    if !delete {
        println!();
        println!(
            "     👉 Run with --delete --unused-for {} to remove unused blobs.",
            age_str
        );
        return Ok(());
    }

    // Count how many manifest entries will be affected
    let manifest_entries_affected = count_manifest_entries_referencing(&stale_hashes);

    // Detailed confirmation with consequences
    if !yes {
        println!();
        println!("  ╔══════════════════════════════════════════════════════╗");
        println!("  ║  ⚠️   DESTRUCTIVE OPERATION — Please review         ║");
        println!("  ╠══════════════════════════════════════════════════════╣");
        println!("  ║                                                      ║");
        println!("  ║  This will:                                          ║");
        println!(
            "  ║    1. Remove {} stale manifest entries  ║",
            format_number(manifest_entries_affected as u64)
        );
        println!(
            "  ║    2. Delete {} CAS blob files          ║",
            format_number(stale_count)
        );
        println!(
            "  ║    3. Reclaim {}                          ║",
            format_bytes(stale_bytes)
        );
        println!("  ║                                                      ║");
        println!("  ║  Consequences:                                       ║");
        println!("  ║    • Affected projects need `vrift ingest` to        ║");
        println!("  ║      restore deleted content                         ║");
        println!("  ║    • VDir cache entries for deleted blobs will        ║");
        println!("  ║      fall back to physical file reads (graceful)     ║");
        println!("  ║    • This operation is NOT reversible                ║");
        println!("  ║                                                      ║");
        println!("  ╚══════════════════════════════════════════════════════╝");
        println!();
        print!("  Proceed? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    // Step 1: Clean manifest entries referencing stale CAS blobs
    if !stale_hashes.is_empty() {
        println!();
        println!("  📦 Step 1/3: Cleaning manifest entries...");
        let removed = cleanup_manifest_entries(&stale_hashes);
        println!(
            "     Removed {} manifest entries",
            format_number(removed as u64)
        );
    }

    // Step 2: Delete CAS blob files
    println!(
        "  🧼 Step 2/3: Deleting {} CAS blobs...",
        format_number(stale_count)
    );

    let del_start = Instant::now();
    let mut deleted_count = 0u64;
    let mut deleted_bytes = 0u64;
    let total = stale_files.len();

    for (i, path) in stale_files.iter().enumerate() {
        // Progress indicator every 10000 files
        if i > 0 && i % 10000 == 0 {
            print!(
                "\r     Progress: {}/{} ({:.0}%)",
                i,
                total,
                (i as f64 / total as f64) * 100.0
            );
            io::stdout().flush().ok();
        }

        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        // Try direct delete first
        match std::fs::remove_file(path) {
            Ok(()) => {
                deleted_count += 1;
                deleted_bytes += file_size;
                continue;
            }
            Err(e) if e.raw_os_error() == Some(libc::EPERM) => {
                // macOS immutable flag — remove uchg then retry
                #[cfg(target_os = "macos")]
                {
                    if clear_immutable_flag(path) && std::fs::remove_file(path).is_ok() {
                        deleted_count += 1;
                        deleted_bytes += file_size;
                        continue;
                    }
                }
                // Also try chmod u+w then delete
                if let Ok(()) = std::fs::set_permissions(
                    path,
                    std::os::unix::fs::PermissionsExt::from_mode(0o644),
                ) {
                    if std::fs::remove_file(path).is_ok() {
                        deleted_count += 1;
                        deleted_bytes += file_size;
                        continue;
                    }
                }
                error_count += 1;
            }
            Err(_) => {
                error_count += 1;
            }
        }
    }
    // Final progress line
    print!("\r     Progress: {}/{} (100%)          ", total, total);
    println!();

    let del_elapsed = del_start.elapsed().as_secs_f64();

    // Step 3: Clean empty directories
    println!("  🧹 Step 3/3: Cleaning empty directories...");
    cleanup_empty_dirs(&blake3_dir);

    println!();
    println!("╔════════════════════════════════════════╗");
    println!("║  ✅ GC Complete in {:.1}s               ║", del_elapsed);
    println!("╚════════════════════════════════════════╝");
    println!();
    println!("   🗑️  {} blobs deleted", format_number(deleted_count));
    println!("   💾 {} reclaimed", format_bytes(deleted_bytes));
    if error_count > 0 {
        println!(
            "   ⚠️  {} files could not be deleted",
            format_number(error_count)
        );
    }
    println!();
    println!("   💡 Run `vrift ingest` in affected projects to restore content.");
    println!();
    Ok(())
}

/// Extract blake3 hash from CAS blob filename.
/// Filename format: "HEXHASH_SIZE.bin" -> [u8; 32]
fn extract_hash_from_cas_filename(path: &Path) -> Option<[u8; 32]> {
    let stem = path.file_stem()?.to_str()?;
    // Format: "abc123..._12345" — hash is before the underscore
    let hash_hex = stem.split('_').next()?;
    if hash_hex.len() != 64 {
        return None;
    }
    let mut hash = [0u8; 32];
    for i in 0..32 {
        hash[i] = u8::from_str_radix(&hash_hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(hash)
}

/// Count manifest entries that reference any of the given stale CAS hashes.
fn count_manifest_entries_referencing(stale_hashes: &HashSet<[u8; 32]>) -> usize {
    let registry = match ManifestRegistry::load_or_create() {
        Ok(r) => r,
        Err(_) => return 0,
    };

    let mut count = 0;
    for entry in registry.manifests.values() {
        if !entry.source_path.exists() {
            continue;
        }
        let lmdb = match LmdbManifest::open(&entry.source_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let entries = match lmdb.iter() {
            Ok(e) => e,
            Err(_) => continue,
        };
        for (_, m_entry) in &entries {
            if stale_hashes.contains(&m_entry.vnode.content_hash) {
                count += 1;
            }
        }
    }
    count
}

/// Remove manifest entries that reference stale CAS blobs, then commit.
fn cleanup_manifest_entries(stale_hashes: &HashSet<[u8; 32]>) -> usize {
    let registry = match ManifestRegistry::load_or_create() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  ⚠️  Could not load manifest registry: {}", e);
            return 0;
        }
    };

    let mut total_removed = 0;
    for entry in registry.manifests.values() {
        if !entry.source_path.exists() {
            continue;
        }
        let lmdb = match LmdbManifest::open(&entry.source_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let entries = match lmdb.iter() {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut removed_in_manifest = 0;
        for (path, m_entry) in &entries {
            if stale_hashes.contains(&m_entry.vnode.content_hash) {
                lmdb.remove(path);
                removed_in_manifest += 1;
            }
        }
        if removed_in_manifest > 0 {
            if let Err(e) = lmdb.commit() {
                eprintln!(
                    "  ⚠️  Failed to commit manifest {:?}: {}",
                    entry.source_path, e
                );
            } else {
                total_removed += removed_in_manifest;
            }
        }
    }
    total_removed
}

/// Remove macOS immutable flag (uchg) from a file using chflags(2)
#[cfg(target_os = "macos")]
fn clear_immutable_flag(path: &Path) -> bool {
    use std::ffi::CString;
    let c_path = match CString::new(path.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // chflags(path, 0) clears all user flags including UF_IMMUTABLE (uchg)
    let ret = unsafe { libc::chflags(c_path.as_ptr(), 0) };
    ret == 0
}

/// Remove empty directories left after blob deletion
fn cleanup_empty_dirs(blake3_dir: &Path) {
    if let Ok(l1_entries) = std::fs::read_dir(blake3_dir) {
        for l1 in l1_entries.flatten() {
            if !l1.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if let Ok(l2_entries) = std::fs::read_dir(l1.path()) {
                for l2 in l2_entries.flatten() {
                    if l2.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        // Remove if empty
                        let _ = std::fs::remove_dir(l2.path());
                    }
                }
            }
            // Remove L1 if empty
            let _ = std::fs::remove_dir(l1.path());
        }
    }
}

#[allow(unused_imports)]
use std::os::unix::fs::PermissionsExt;

/// Format bytes in human-readable form
pub(crate) fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Format number with comma separators
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
