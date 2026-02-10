//! # Garbage Collection (RFC-0041)
//!
//! Multi-manifest garbage collection with registry integration and Bloom-assisted daemon sweep.

use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
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
}

pub async fn run(cas_root: &Path, args: GcArgs) -> Result<()> {
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
