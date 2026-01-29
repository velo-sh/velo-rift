//! # velo CLI
//!
//! Command-line interface for Velo Rift content-addressable filesystem.
//!
//! ## Commands
//!
//! - `velo ingest <dir>` - Import files to CAS and generate manifest
//! - `velo run <cmd>` - Execute command with LD_PRELOAD (placeholder)
//! - `velo status` - Display CAS statistics

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use walkdir::WalkDir;

mod isolation;

use velo_cas::CasStore;
use velo_manifest::{Manifest, VnodeEntry};

/// Velo Rift - Content-Addressable Virtual Filesystem
#[derive(Parser)]
#[command(name = "velo")]
#[command(version, about, long_about = None)]
struct Cli {
    /// CAS storage root directory
    #[arg(long, env = "VELO_CAS_ROOT", default_value = "/var/velo/the_source")]
    cas_root: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Import files from a directory into the CAS
    Ingest {
        /// Directory to ingest
        #[arg(value_name = "DIR")]
        directory: PathBuf,

        /// Output manifest file path
        #[arg(short, long, default_value = "velo.manifest")]
        output: PathBuf,

        /// Base path prefix in manifest (default: use directory name)
        #[arg(short, long)]
        prefix: Option<String>,
    },

    /// Execute a command with Velo VFS (placeholder)
    Run {
        /// Manifest file to use
        #[arg(short, long, default_value = "velo.manifest")]
        manifest: PathBuf,

        /// Command to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,

        /// Enable Linux namespace isolation
        #[arg(long)]
        isolate: bool,
    },

    /// Display CAS statistics
    Status {
        /// Also show manifest statistics if a manifest file is provided
        #[arg(short, long)]
        manifest: Option<PathBuf>,
    },

    /// Mount the manifest as a FUSE filesystem
    Mount {
        /// Manifest file to mount
        #[arg(short, long, default_value = "velo.manifest")]
        manifest: PathBuf,

        /// Mount point directory
        #[arg(value_name = "MOUNTPOINT")]
        mountpoint: PathBuf,
    },

    /// Resolve dependencies from a velo.lock file
    Resolve {
        /// Lockfile path
        #[arg(short, long, default_value = "velo.lock")]
        lockfile: PathBuf,
    },

    /// Garbage Collection
    Gc {
        /// Directory containing manifests to scan for active blobs
        #[arg(long, value_name = "MANIFEST_DIR")]
        manifests: PathBuf,

        /// Delete garbage blobs (default: dry-run)
        #[arg(long)]
        delete: bool,

        /// Print deleted blob hashes
        #[arg(long)]
        verbose: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Ingest {
            directory,
            output,
            prefix,
        } => cmd_ingest(&cli.cas_root, &directory, &output, prefix.as_deref()),
        Commands::Run {
            manifest,
            command,
            isolate,
        } => cmd_run(&cli.cas_root, &manifest, &command, isolate),
        Commands::Status { manifest } => cmd_status(&cli.cas_root, manifest.as_deref()),
        Commands::Mount {
            manifest,
            mountpoint,
        } => cmd_mount(&cli.cas_root, &manifest, &mountpoint),
        Commands::Resolve { lockfile } => cmd_resolve(&cli.cas_root, &lockfile),
        Commands::Gc {
            manifests,
            delete,
            verbose,
        } => cmd_gc(&cli.cas_root, &manifests, delete, verbose),
    }
}

/// Run Garbage Collection
fn cmd_gc(cas_root: &Path, manifest_dir: &Path, delete: bool, verbose: bool) -> Result<()> {
    use std::collections::HashSet;
    use velo_cas::CasStore;

    if !cas_root.exists() {
        anyhow::bail!("CAS root not found: {}", cas_root.display());
    }
    if !manifest_dir.exists() {
        anyhow::bail!("Manifest directory not found: {}", manifest_dir.display());
    }

    println!("Starting Garbage Collection...");
    if !delete {
        println!("Mode: DRY RUN (no files will be deleted)");
    } else {
        println!("Mode: DELETE (garbage files will be removed)");
    }

    // 1. Mark Phase: Scan manifests for active blobs
    println!("Scanning manifests in {}...", manifest_dir.display());
    let mut active_blobs = HashSet::new();
    let mut manifest_count = 0;

    for entry in fs::read_dir(manifest_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "bin" || ext == "manifest")
        {
            // Try to load manifest
            match Manifest::load(&path) {
                Ok(manifest) => {
                    manifest_count += 1;
                    for (_, vnode) in manifest.iter() {
                        active_blobs.insert(vnode.content_hash);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to load manifest {}: {}", path.display(), e);
                }
            }
        }
    }
    println!("  Scanned {} manifests.", manifest_count);
    println!("  Found {} active unique blobs.", active_blobs.len());

    // 2. Sweep Phase: Scan CAS for unused blobs
    println!("Scanning CAS {}...", cas_root.display());
    let cas = CasStore::new(cas_root)?;

    let mut garbage_count = 0;
    let mut garbage_bytes = 0u64;
    let mut kept_count = 0;

    for blob_hash_res in cas.iter()? {
        let blob_hash = blob_hash_res?;

        if active_blobs.contains(&blob_hash) {
            kept_count += 1;
        } else {
            // Garbage found!
            // Get size for stats
            let size = if let Some(path) = cas.blob_path_for_hash(&blob_hash) {
                fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };

            garbage_count += 1;
            garbage_bytes += size;

            if verbose {
                println!("  [GARBAGE] {}", CasStore::hash_to_hex(&blob_hash));
            }

            if delete {
                if let Some(path) = cas.blob_path_for_hash(&blob_hash) {
                    if let Err(e) = fs::remove_file(&path) {
                        eprintln!("Failed to delete {}: {}", path.display(), e);
                    }
                }
            }
        }
    }

    println!("\nGC Summary:");
    println!("  Active blobs:   {}", kept_count);
    println!("  Garbage blobs:  {}", garbage_count);
    println!("  Garbage size:   {}", format_bytes(garbage_bytes));

    if !delete && garbage_count > 0 {
        println!("\nRun with --delete to reclaim this space.");
    }

    Ok(())
}

/// Resolve dependencies from a lockfile
fn cmd_resolve(cas_root: &Path, lockfile: &Path) -> Result<()> {
    if !lockfile.exists() {
        anyhow::bail!("Lockfile not found: {}", lockfile.display());
    }

    println!("Resolving lockfile: {}", lockfile.display());
    let lock = velo_lock::VeloLock::load(lockfile)?;

    println!("  Engine: {}", lock.meta.engine);
    println!("  Target: {}", lock.meta.target_platform);
    println!("  Packages: {}", lock.packages.len());

    let cas = CasStore::new(cas_root)?;
    let mut missing = 0;
    let mut resolved = 0;

    println!("\nVerifying CAS content...");

    for (name, pkg) in &lock.packages {
        // Parse "tree:hex_hash" format
        if let Some(hash_str) = pkg.source_tree.strip_prefix("tree:") {
            if let Some(hash) = CasStore::hex_to_hash(hash_str) {
                if cas.exists(&hash) {
                    resolved += 1;
                } else {
                    println!("  [MISSING] {} v{} (Tree: {})", name, pkg.version, hash_str);
                    missing += 1;
                }
            } else {
                println!(
                    "  [INVALID] {} v{} (Bad hash: {})",
                    name, pkg.version, hash_str
                );
                missing += 1;
            }
        } else {
            println!(
                "  [INVALID] {} v{} (Bad prefix: {})",
                name, pkg.version, pkg.source_tree
            );
            missing += 1;
        }
    }

    println!("\nResult: {} resolved, {} missing", resolved, missing);

    if missing > 0 {
        println!("Note: In a full implementation, this command would fetch missing trees from L2 storage.");
        // In MVP, we just report them.
    }

    Ok(())
}

/// Mount the Velo filesystem (requires FUSE)
fn cmd_mount(cas_root: &Path, manifest: &Path, mountpoint: &Path) -> Result<()> {
    if !manifest.exists() {
        anyhow::bail!("Manifest not found: {}", manifest.display());
    }

    // Ensure mountpoint exists
    if !mountpoint.exists() {
        fs::create_dir_all(mountpoint)?;
    }

    println!("Mounting VeloFS...");
    println!("  Manifest:   {}", manifest.display());
    println!("  CAS:        {}", cas_root.display());
    println!("  Mountpoint: {}", mountpoint.display());

    #[cfg(feature = "fuse")]
    {
        let cas = CasStore::new(cas_root)?;
        let manifest = Manifest::load(manifest)?;
        let _fs = velo_fuse::VeloFs::new(&manifest, cas);
        // fuser::mount2(_fs, mountpoint, &[])?;
        println!("FUSE mount implemented but requires 'fuse' feature enabled in velo-cli");
    }

    #[cfg(not(feature = "fuse"))]
    {
        println!("⚠️  FUSE support disabled. Recompile with --features fuse to enable.");
    }

    Ok(())
}

/// Ingest a directory into the CAS and create a manifest
fn cmd_ingest(
    cas_root: &Path,
    directory: &Path,
    output: &Path,
    prefix: Option<&str>,
) -> Result<()> {
    // Validate input directory
    if !directory.exists() {
        anyhow::bail!("Directory does not exist: {}", directory.display());
    }
    if !directory.is_dir() {
        anyhow::bail!("Not a directory: {}", directory.display());
    }

    // Initialize CAS store
    let cas = CasStore::new(cas_root)
        .with_context(|| format!("Failed to initialize CAS at {}", cas_root.display()))?;

    // Determine path prefix
    let base_prefix = prefix.unwrap_or_else(|| {
        directory
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("root")
    });

    let mut manifest = Manifest::new();
    let mut files_ingested = 0u64;
    let mut bytes_ingested = 0u64;
    let mut unique_blobs = 0u64;

    println!("Ingesting {} into CAS...", directory.display());

    for entry in WalkDir::new(directory).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let relative = path.strip_prefix(directory).unwrap_or(path);

        // Build manifest path
        let manifest_path = if relative.as_os_str().is_empty() {
            format!("/{}", base_prefix)
        } else {
            format!("/{}/{}", base_prefix, relative.display())
        };

        let metadata = fs::metadata(path)?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if metadata.is_dir() {
            let vnode = VnodeEntry::new_directory(mtime, metadata.mode());
            manifest.insert(&manifest_path, vnode);
        } else if metadata.is_file() {
            // Store file content in CAS
            let content = fs::read(path)
                .with_context(|| format!("Failed to read file: {}", path.display()))?;

            let was_new = !cas.exists(&CasStore::compute_hash(&content));
            let hash = cas.store(&content)?;

            if was_new {
                unique_blobs += 1;
            }

            let vnode = VnodeEntry::new_file(hash, metadata.len(), mtime, metadata.mode());
            manifest.insert(&manifest_path, vnode);

            files_ingested += 1;
            bytes_ingested += metadata.len();
        }
    }

    // Save manifest
    manifest
        .save(output)
        .with_context(|| format!("Failed to save manifest to {}", output.display()))?;

    let stats = manifest.stats();
    let dedup_ratio = if files_ingested > 0 {
        100.0 * (1.0 - (unique_blobs as f64 / files_ingested as f64))
    } else {
        0.0
    };

    println!("\n✓ Ingestion complete");
    println!("  Files:       {}", stats.file_count);
    println!("  Directories: {}", stats.dir_count);
    println!("  Total size:  {} bytes", format_bytes(bytes_ingested));
    println!(
        "  Unique blobs: {} ({:.1}% dedup)",
        unique_blobs, dedup_ratio
    );
    println!("  Manifest:    {}", output.display());

    Ok(())
}

/// Execute a command with Velo VFS shim
fn cmd_run(cas_root: &Path, manifest: &Path, command: &[String], isolate: bool) -> Result<()> {
    if command.is_empty() {
        anyhow::bail!("No command specified");
    }

    if !manifest.exists() {
        anyhow::bail!("Manifest not found: {}", manifest.display());
    }

    // Handle isolation if requested
    if isolate {
        return isolation::run_isolated(command, manifest, cas_root);
    }

    // Standard LD_PRELOAD execution
    // Find the shim library
    let shim_path = find_shim_library()?;

    let manifest_abs = manifest
        .canonicalize()
        .with_context(|| format!("Failed to resolve manifest path: {}", manifest.display()))?;
    let cas_abs = cas_root
        .canonicalize()
        .unwrap_or_else(|_| cas_root.to_path_buf());

    println!("Running with Velo VFS:");
    println!("  Shim:     {}", shim_path.display());
    println!("  Manifest: {}", manifest_abs.display());
    println!("  CAS:      {}", cas_abs.display());
    println!("  Command:  {}", command.join(" "));
    println!();

    // Build the command with environment variables
    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);

    // Set Velo environment variables
    cmd.env("VELO_MANIFEST", &manifest_abs);
    cmd.env("VELO_CAS_ROOT", &cas_abs);

    // Set platform-specific library preload
    #[cfg(target_os = "macos")]
    {
        cmd.env("DYLD_INSERT_LIBRARIES", &shim_path);
        // Disable SIP restrictions for child process (requires entitlements in production)
        cmd.env("DYLD_FORCE_FLAT_NAMESPACE", "1");
    }

    #[cfg(target_os = "linux")]
    {
        cmd.env("LD_PRELOAD", &shim_path);
    }

    // Enable debug output if VELO_DEBUG is set
    if std::env::var("VELO_DEBUG").is_ok() {
        cmd.env("VELO_DEBUG", "1");
    }

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute: {}", command[0]))?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Find the velo-shim library
fn find_shim_library() -> Result<PathBuf> {
    // Check standard locations
    let candidates = [
        // Development: relative to cargo target
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .map(|p| {
                #[cfg(target_os = "macos")]
                {
                    p.join("libvelo_shim.dylib")
                }
                #[cfg(target_os = "linux")]
                {
                    p.join("libvelo_shim.so")
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    p.join("libvelo_shim.so")
                }
            }),
        // Installed location
        Some(PathBuf::from("/usr/local/lib/velo/libvelo_shim.so")),
        #[cfg(target_os = "macos")]
        Some(PathBuf::from("/usr/local/lib/velo/libvelo_shim.dylib")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Provide helpful error message
    anyhow::bail!(
        "Could not find velo-shim library. \n\
        Build with: cargo build -p velo-shim --release\n\
        Expected at: target/release/libvelo_shim.{}",
        if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        }
    );
}

/// Display CAS and optionally manifest statistics
fn cmd_status(cas_root: &Path, manifest: Option<&Path>) -> Result<()> {
    println!("Velo Rift Status");
    println!("================");
    println!();

    // CAS statistics
    if cas_root.exists() {
        let cas = CasStore::new(cas_root)?;
        let stats = cas.stats()?;

        println!("CAS Store: {}", cas_root.display());
        println!("  Unique blobs: {}", stats.blob_count);
        println!("  Total size:   {}", format_bytes(stats.total_bytes));
        println!("  Avg blob:     {}", format_bytes(stats.avg_blob_size()));
        println!();
        println!("  Size distribution:");
        println!("    <1KB:      {} blobs", stats.small_blobs);
        println!("    1KB-1MB:   {} blobs", stats.medium_blobs);
        println!("    1MB-100MB: {} blobs", stats.large_blobs);
        println!("    >100MB:    {} blobs", stats.huge_blobs);
    } else {
        println!("CAS Store: {} (not initialized)", cas_root.display());
    }

    // Manifest statistics with dedup calculation
    if let Some(manifest_path) = manifest {
        println!();
        if manifest_path.exists() {
            let manifest = Manifest::load(manifest_path)?;
            let mstats = manifest.stats();

            println!("Manifest: {}", manifest_path.display());
            println!("  Files:       {}", mstats.file_count);
            println!("  Directories: {}", mstats.dir_count);
            println!("  Total size:  {}", format_bytes(mstats.total_size));

            // Calculate dedup ratio if CAS is available
            if cas_root.exists() {
                let cas = CasStore::new(cas_root)?;
                let cas_stats = cas.stats()?;
                if mstats.total_size > 0 && cas_stats.total_bytes > 0 {
                    let savings = mstats.total_size.saturating_sub(cas_stats.total_bytes);
                    let ratio = (savings as f64 / mstats.total_size as f64) * 100.0;
                    println!();
                    println!("  Deduplication:");
                    println!("    Original:     {}", format_bytes(mstats.total_size));
                    println!("    Deduplicated: {}", format_bytes(cas_stats.total_bytes));
                    println!(
                        "    Savings:      {} ({:.1}%)",
                        format_bytes(savings),
                        ratio
                    );
                }
            }
        } else {
            println!("Manifest: {} (not found)", manifest_path.display());
        }
    }

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
        format!("{} B", bytes)
    }
}
