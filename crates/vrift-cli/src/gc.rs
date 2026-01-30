use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashSet;
use std::path::PathBuf;
use vrift_cas::CasStore;
use vrift_manifest::Manifest;

#[derive(Args, Debug)]
pub struct GcArgs {
    /// Path to the Manifest file to treat as the "root" of live objects.
    #[arg(long, default_value = "vrift.manifest")]
    manifest: PathBuf,

    /// Actually delete blobs (default is dry-run)
    #[arg(long)]
    delete: bool,
}

pub fn run(args: GcArgs) -> Result<()> {
    println!("[*] Starting Garbage Collection...");
    
    if args.delete {
        println!("!! WARNING: DELETING UNREFERENCED DATA !!");
    } else {
         println!("(Dry Run Only: use --delete to remove files)");
    }

    // 1. Mark: Load Manifest and find all live hashes
    println!(" -> Loading manifest from {:?}", args.manifest);
    // TODO: support multiple manifests? For now single source of truth.
    let manifest = Manifest::load(&args.manifest)
        .context("Failed to parse manifest")?;

    let mut keep_set = HashSet::new();
    for (_, entry) in manifest.iter() {
        keep_set.insert(entry.content_hash);
    }
    println!(" -> Found {} live blobs referenced by manifest.", keep_set.len());

    // 2. Sweep: Iterate CAS and find orphans
    let cas = CasStore::default_location()?;
    let mut total_blobs = 0;
    let mut reclaimed_bytes = 0;
    let mut deleted_count = 0;
    let mut dry_run_deleted_count = 0;

    println!(" -> Scanning CAS at {:?}...", cas.root());
    
    // We collect orphans first to avoid modifying iteration (though iterator relies on dir listing, deleting might be safe, but safer to collect)
    let mut orphans = Vec::new();

    for hash_res in cas.iter()? {
        let hash = hash_res?;
        total_blobs += 1;

        if !keep_set.contains(&hash) {
            orphans.push(hash);
        }
    }

    println!(" -> Scan complete. Total blobs: {}. Orphans found: {}.", total_blobs, orphans.len());

    // 3. Delete Orphans
    for hash in orphans {
        // Get size for reporting
        let size = if let Ok(blob) = cas.get(&hash) {
             blob.len() as u64
        } else {
             0
        };

        if args.delete {
             match cas.delete(&hash) {
                Ok(_) => {
                     reclaimed_bytes += size;
                     deleted_count += 1;
                }
                Err(e) => {
                    eprintln!("Failed to delete blob {}: {}", vrift_cas::CasStore::hash_to_hex(&hash), e);
                }
            }
        } else {
            println!(" [Dry Run] would delete: {} ({})", vrift_cas::CasStore::hash_to_hex(&hash), size);
            reclaimed_bytes += size;
            dry_run_deleted_count += 1;
        }
    }

    println!("\n[GC summary]");
    println!("  Total Scanned:   {}", total_blobs);
    if args.delete {
        println!("  Deleted:         {}", deleted_count);
        println!("  Reclaimed:       {} bytes", reclaimed_bytes);
    } else {
        println!("  Can Delete:      {}", dry_run_deleted_count);
        println!("  Potential Space: {} bytes", reclaimed_bytes);
        println!("\nTo perform deletion, run with --delete");
    }

    Ok(())
}
