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
use vrift_cas::{physical_size, CasStore};
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
                            orphan_bytes += physical_size(&meta);
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
                    stale_bytes += physical_size(&meta);
                    if delete {
                        // Extract blake3 hash from filename: "HASH_SIZE.bin" -> HASH
                        if let Some(hash) = extract_hash_from_cas_filename(&path) {
                            stale_hashes.insert(hash);
                        }
                        stale_files.push(path);
                    }
                } else {
                    kept_count += 1;
                    kept_bytes += physical_size(&meta);
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

        let file_size = std::fs::metadata(path)
            .map(|m| physical_size(&m))
            .unwrap_or(0);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    // ========================================================================
    // parse_duration tests
    // ========================================================================

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("24h").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(
            parse_duration("0h").err().unwrap(),
            "Duration must be greater than zero"
        );
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(
            parse_duration("7d").unwrap(),
            Duration::from_secs(7 * 86400)
        );
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(
            parse_duration("30d").unwrap(),
            Duration::from_secs(30 * 86400)
        );
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("90m").unwrap(), Duration::from_secs(5400));
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("3600s").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_compound() {
        // 1h30m = 5400s
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        // 1d12h = 36h = 129600s
        assert_eq!(
            parse_duration("1d12h").unwrap(),
            Duration::from_secs(129600)
        );
        // 2h30m15s = 9015s
        assert_eq!(
            parse_duration("2h30m15s").unwrap(),
            Duration::from_secs(9015)
        );
    }

    #[test]
    fn test_parse_duration_bare_number_defaults_to_hours() {
        // Bare number defaults to hours
        assert_eq!(parse_duration("24").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("1").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_whitespace() {
        assert_eq!(
            parse_duration("  24h  ").unwrap(),
            Duration::from_secs(86400)
        );
    }

    #[test]
    fn test_parse_duration_zero_rejected() {
        assert!(parse_duration("0h").is_err());
        assert!(parse_duration("0d").is_err());
        assert!(parse_duration("0").is_err());
    }

    #[test]
    fn test_parse_duration_invalid_unit() {
        assert!(parse_duration("24x").is_err());
        assert!(parse_duration("5w").is_err()); // weeks not supported
    }

    #[test]
    fn test_parse_duration_empty() {
        assert!(parse_duration("").is_err());
    }

    // ========================================================================
    // extract_hash_from_cas_filename tests
    // ========================================================================

    #[test]
    fn test_extract_hash_valid() {
        // Standard CAS filename: HASH_SIZE.bin
        let path = PathBuf::from(
            "/cas/blake3/ab/cd/abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789_1024.bin",
        );
        let hash = extract_hash_from_cas_filename(&path).unwrap();
        assert_eq!(hash[0], 0xab);
        assert_eq!(hash[1], 0xcd);
        assert_eq!(hash[2], 0xef);
        assert_eq!(hash[31], 0x89);
    }

    #[test]
    fn test_extract_hash_all_zeros() {
        let path = PathBuf::from(
            "/cas/blake3/00/00/0000000000000000000000000000000000000000000000000000000000000000_42.bin",
        );
        let hash = extract_hash_from_cas_filename(&path).unwrap();
        assert_eq!(hash, [0u8; 32]);
    }

    #[test]
    fn test_extract_hash_all_ff() {
        let path = PathBuf::from(
            "/cas/blake3/ff/ff/ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff_99.bin",
        );
        let hash = extract_hash_from_cas_filename(&path).unwrap();
        assert_eq!(hash, [0xff; 32]);
    }

    #[test]
    fn test_extract_hash_short_hash_rejected() {
        // Hash too short (only 62 hex chars)
        let path = PathBuf::from("/cas/blake3/ab/cd/abcdef01234567890123456789012345678901234567890123456789012345_1024.bin");
        assert!(extract_hash_from_cas_filename(&path).is_none());
    }

    #[test]
    fn test_extract_hash_no_underscore() {
        let path = PathBuf::from("/cas/blake3/ab/cd/abcdef.bin");
        assert!(extract_hash_from_cas_filename(&path).is_none());
    }

    #[test]
    fn test_extract_hash_no_extension() {
        // file_stem() returns None for files with only extension
        let path = PathBuf::from("/cas/blake3/ab/cd/.hidden");
        assert!(extract_hash_from_cas_filename(&path).is_none());
    }

    #[test]
    fn test_extract_hash_invalid_hex() {
        // 'gg' is not valid hex
        let path = PathBuf::from(
            "/cas/blake3/ab/cd/ggcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789_1024.bin",
        );
        assert!(extract_hash_from_cas_filename(&path).is_none());
    }

    // ========================================================================
    // format_bytes tests
    // ========================================================================

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(1023), "1023 bytes");
    }

    #[test]
    fn test_format_bytes_kb() {
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
    }

    #[test]
    fn test_format_bytes_mb() {
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.00 MB");
    }

    #[test]
    fn test_format_bytes_gb() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(
            format_bytes(141 * 1024 * 1024 * 1024 + 737 * 1024 * 1024),
            "141.72 GB"
        );
    }

    // ========================================================================
    // format_number tests
    // ========================================================================

    #[test]
    fn test_format_number_small() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(1), "1");
        assert_eq!(format_number(999), "999");
    }

    #[test]
    fn test_format_number_thousands() {
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(19806), "19,806");
        assert_eq!(format_number(315853), "315,853");
    }

    #[test]
    fn test_format_number_millions() {
        assert_eq!(format_number(1000000), "1,000,000");
        assert_eq!(format_number(1234567), "1,234,567");
    }

    // ========================================================================
    // Time-based GC filesystem tests
    // ========================================================================

    /// Helper: create a fake CAS directory structure with blobs at specific ages
    fn create_test_cas(dir: &Path, entries: &[(&str, u64, Duration)]) {
        // entries: (hash_hex, size_bytes, age)
        for (hash_hex, size, age) in entries {
            let prefix1 = &hash_hex[..2];
            let prefix2 = &hash_hex[2..4];
            let blob_dir = dir.join("blake3").join(prefix1).join(prefix2);
            std::fs::create_dir_all(&blob_dir).unwrap();

            let filename = format!("{}_{}.bin", hash_hex, size);
            let blob_path = blob_dir.join(&filename);
            let content = vec![0u8; *size as usize];
            std::fs::write(&blob_path, &content).unwrap();

            // Set BOTH atime and mtime to `age` ago.
            // If we only set mtime, macOS may update atime to "now" when
            // the file is read during scan, making max(atime, mtime) = now.
            let ft = filetime::FileTime::from_system_time(SystemTime::now() - *age);
            filetime::set_file_times(&blob_path, ft, ft).unwrap();
        }
    }

    #[test]
    fn test_scan_separates_stale_and_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let cas_root = tmp.path();

        // Create blobs: 2 old (48h), 1 fresh (1h)
        create_test_cas(
            cas_root,
            &[
                (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    100,
                    Duration::from_secs(48 * 3600),
                ),
                (
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    200,
                    Duration::from_secs(48 * 3600),
                ),
                (
                    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    300,
                    Duration::from_secs(3600),
                ),
            ],
        );

        // Scan with 24h cutoff
        let cutoff = SystemTime::now() - Duration::from_secs(24 * 3600);
        let blake3_dir = cas_root.join("blake3");

        let mut stale_count = 0u64;
        let mut stale_bytes = 0u64;
        let mut fresh_count = 0u64;

        let l1_entries = std::fs::read_dir(&blake3_dir).unwrap();
        for l1 in l1_entries.flatten() {
            if !l1.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            for l2 in std::fs::read_dir(l1.path()).unwrap().flatten() {
                if !l2.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                for l3 in std::fs::read_dir(l2.path()).unwrap().flatten() {
                    let meta = std::fs::metadata(l3.path()).unwrap();
                    if !meta.is_file() {
                        continue;
                    }
                    let mtime = meta.modified().unwrap();
                    let atime = meta.accessed().unwrap_or(mtime);
                    let last_used = std::cmp::max(atime, mtime);
                    if last_used < cutoff {
                        stale_count += 1;
                        stale_bytes += physical_size(&meta);
                    } else {
                        fresh_count += 1;
                    }
                }
            }
        }

        assert_eq!(stale_count, 2, "Expected 2 stale blobs");
        // physical_size returns block-aligned values (st_blocks * 512),
        // so exact byte totals won't match — just verify it's > 0
        assert!(stale_bytes > 0, "Expected non-zero stale bytes");
        assert_eq!(fresh_count, 1, "Expected 1 fresh blob");
    }

    #[test]
    fn test_scan_all_fresh_nothing_to_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let cas_root = tmp.path();

        // All files are fresh (created 1h ago)
        create_test_cas(
            cas_root,
            &[
                (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    100,
                    Duration::from_secs(3600),
                ),
                (
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    200,
                    Duration::from_secs(1800),
                ),
            ],
        );

        let cutoff = SystemTime::now() - Duration::from_secs(24 * 3600);
        let blake3_dir = cas_root.join("blake3");
        let mut stale_count = 0u64;

        for l1 in std::fs::read_dir(&blake3_dir).unwrap().flatten() {
            if !l1.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            for l2 in std::fs::read_dir(l1.path()).unwrap().flatten() {
                if !l2.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                for l3 in std::fs::read_dir(l2.path()).unwrap().flatten() {
                    let meta = std::fs::metadata(l3.path()).unwrap();
                    if !meta.is_file() {
                        continue;
                    }
                    let mtime = meta.modified().unwrap();
                    if mtime < cutoff {
                        stale_count += 1;
                    }
                }
            }
        }

        assert_eq!(stale_count, 0, "All files should be fresh");
    }

    #[test]
    fn test_cleanup_empty_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let blake3_dir = tmp.path().join("blake3");
        let nested = blake3_dir.join("aa").join("bb");
        std::fs::create_dir_all(&nested).unwrap();
        // Write a file in a different bucket
        let other = blake3_dir.join("cc").join("dd");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("file.bin"), b"data").unwrap();

        // Empty dir aa/bb should be removed, cc/dd should remain
        cleanup_empty_dirs(&blake3_dir);

        assert!(
            !blake3_dir.join("aa").exists(),
            "Empty L1 dir should be removed"
        );
        assert!(
            blake3_dir.join("cc").join("dd").exists(),
            "Non-empty dir should remain"
        );
        assert!(
            blake3_dir.join("cc").join("dd").join("file.bin").exists(),
            "File should remain"
        );
    }

    #[test]
    fn test_extract_and_delete_roundtrip() {
        // Verify extract_hash_from_cas_filename produces correct hash that
        // can be matched against manifest entries
        let hash_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let path = PathBuf::from(format!("/cas/blake3/01/23/{}_{}.bin", hash_hex, 1024));
        let extracted = extract_hash_from_cas_filename(&path).unwrap();

        // Verify roundtrip: hex -> bytes -> hex
        let roundtrip: String = extracted.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(roundtrip, hash_hex);
    }
}
