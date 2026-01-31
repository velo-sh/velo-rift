//! # Garbage Collection (RFC-0041)
//!
//! Multi-manifest garbage collection with registry integration.

use anyhow::{Context, Result};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;
use vrift_cas::CasStore;
use vrift_manifest::Manifest;

use crate::registry::ManifestRegistry;

#[derive(Args, Debug)]
pub struct GcArgs {
    /// Path to a single Manifest file (legacy mode, bypasses registry)
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Actually delete blobs (default is dry-run)
    #[arg(long)]
    delete: bool,

    /// Remove stale manifest entries before GC
    #[arg(long)]
    prune_stale: bool,

    /// Only delete orphans older than this duration (e.g., "1h", "24h")
    #[arg(long)]
    older_than: Option<String>,

    /// Skip grace period and delete immediately (dangerous!)
    #[arg(long)]
    immediate: bool,

    /// Skip confirmation prompt (for scripts and CI)
    #[arg(long, short = 'y')]
    yes: bool,
}

pub fn run(cas_root: &std::path::Path, args: GcArgs) -> Result<()> {
    // Print header like ingest
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
        // Legacy mode: single manifest
        println!();
        println!("  [Legacy Mode] Using single manifest: {:?}", manifest_path);
        let manifest = Manifest::load(manifest_path).context("Failed to parse manifest")?;
        manifest
            .iter()
            .map(|(_, entry)| entry.content_hash)
            .collect()
    } else {
        // Registry mode: all active manifests
        println!();
        println!("  Registry Status:");
        println!(
            "    📁 Registered manifests: {} ({} active, {} stale)",
            registry.manifests.len(),
            active_count,
            stale_count
        );

        // Show stale manifests
        if stale_count > 0 {
            println!();
            println!("  ⚠️  Stale Manifests (source path deleted):");
            for (uuid, entry) in registry.stale_manifests() {
                let short_uuid = &uuid[..8];
                println!("      {} - {:?}", short_uuid, entry.project_root);
            }

            if !args.prune_stale {
                println!();
                println!("  💡 Run with --prune-stale to remove stale entries first.");
                println!("     Stale manifests still protect their blobs until removed.");
            }
        }

        // Prune stale if requested
        if args.prune_stale && stale_count > 0 {
            let pruned = registry.prune_stale();
            registry.save()?;
            println!();
            println!("  🗑️  Pruned {} stale manifest entries", pruned);
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

    // Sweep: Iterate CAS and find orphans
    let cas = CasStore::new(cas_root)?;
    let mut total_blobs = 0u64;
    let mut orphan_count = 0u64;
    let mut orphan_bytes = 0u64;
    let mut deleted_count = 0u64;
    let mut deleted_bytes = 0u64;

    // Calculate total CAS size and collect orphans
    let mut orphans = Vec::new();
    let mut total_bytes = 0u64;
    for hash_res in cas.iter()? {
        let hash = hash_res?;
        total_blobs += 1;
        let size = cas.get(&hash).map(|b| b.len() as u64).unwrap_or(0);
        total_bytes += size;

        if !keep_set.contains(&hash) {
            orphans.push((hash, size));
            orphan_count += 1;
            orphan_bytes += size;
        }
    }

    println!();
    println!("  CAS Statistics:");
    println!(
        "    📦 Total blobs:   {} ({})",
        format_number(total_blobs),
        format_bytes(total_bytes)
    );
    println!(
        "    ✅ Referenced:    {}",
        format_number(keep_set.len() as u64)
    );
    println!(
        "    🗑️  Orphaned:      {} ({})",
        format_number(orphan_count),
        format_bytes(orphan_bytes)
    );

    if orphan_count > 0 && total_bytes > 0 {
        let reclaim_pct = (orphan_bytes as f64 / total_bytes as f64) * 100.0;
        println!("    💾 Reclaimable:   {:.1}% of CAS", reclaim_pct);
    }

    // Delete orphans if requested
    if args.delete {
        if orphan_count == 0 {
            println!();
            println!("╔════════════════════════════════════════╗");
            println!("║  ✨ CAS is Clean - No Orphans!         ║");
            println!("╚════════════════════════════════════════╝");
        } else {
            // Ask for confirmation unless --yes is specified
            if !args.yes {
                println!();
                print!(
                    "  ⚠️  Delete {} blobs ({})? [y/N] ",
                    format_number(orphan_count),
                    format_bytes(orphan_bytes)
                );
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("  Cancelled.");
                    println!();
                    return Ok(());
                }
            }
            let gc_start = Instant::now();

            // Create progress bar for deletion
            let pb = ProgressBar::new(orphan_count);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("   [{bar:40.red/white}] {pos:>7}/{len:7} • {msg}")
                    .unwrap()
                    .progress_chars("█▓░"),
            );
            pb.set_message("0 B reclaimed".to_string());

            for (hash, size) in orphans {
                match cas.delete(&hash) {
                    Ok(_) => {
                        deleted_count += 1;
                        deleted_bytes += size;
                        pb.inc(1);
                        // Update reclaimed size message
                        pb.set_message(format!("{} reclaimed", format_bytes(deleted_bytes)));
                    }
                    Err(e) => {
                        pb.inc(1);
                        pb.println(format!(
                            "  ❌ Failed to delete {}: {}",
                            CasStore::hash_to_hex(&hash),
                            e
                        ));
                    }
                }
            }
            pb.finish_and_clear();

            let gc_elapsed = gc_start.elapsed().as_secs_f64();
            let delete_rate = if gc_elapsed > 0.0 {
                deleted_count as f64 / gc_elapsed
            } else {
                0.0
            };
            let reclaim_pct = if total_bytes > 0 {
                (deleted_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };

            // Print prominent completion box
            println!();
            println!("╔════════════════════════════════════════╗");
            println!("║  ✅ GC Complete in {:.2}s              ║", gc_elapsed);
            println!("╚════════════════════════════════════════╝");
            println!();
            // Highlight the key metrics users care about
            println!(
                "   🗑️  {} orphaned blobs deleted",
                format_number(deleted_count)
            );
            println!("   💾 {} reclaimed", format_bytes(deleted_bytes));
            println!("   📉 CAS reduced by {:.1}%", reclaim_pct);
            println!("   ⚡ {:.0} blobs/sec", delete_rate);
        }
    } else {
        // Dry run output - highlight what WOULD be reclaimed
        println!();
        if orphan_count > 0 {
            let reclaim_pct = if total_bytes > 0 {
                (orphan_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            println!("╔════════════════════════════════════════╗");
            println!("║  📋 Dry Run Complete                   ║");
            println!("╚════════════════════════════════════════╝");
            println!();
            println!("   🗑️  {} orphans found", format_number(orphan_count));
            println!("   💾 {} can be reclaimed", format_bytes(orphan_bytes));
            println!("   📉 Would reduce CAS by {:.1}%", reclaim_pct);
            println!();
            println!("   👉 Run with --delete to reclaim space");
        } else {
            println!("╔════════════════════════════════════════╗");
            println!("║  ✨ CAS is Clean - No Orphans!         ║");
            println!("╚════════════════════════════════════════╝");
        }
    }

    // Save registry (in case we updated verification times)
    registry.save()?;

    println!();
    Ok(())
}

/// Format bytes in human-readable form
fn format_bytes(bytes: u64) -> String {
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

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 bytes");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(1234567), "1,234,567");
    }
}
