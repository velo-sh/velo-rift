//! # vrift CLI
//!
//! Command-line interface for Velo Rift content-addressable filesystem.
//!
//! ## Commands
//!
//! - `vrift ingest <dir>` - Import files to CAS and generate manifest
//! - `vrift run <cmd>` - Execute command with VeloVFS virtualization
//! - `vrift status` - Display CAS statistics

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use walkdir::WalkDir;

mod active;
mod daemon;
pub mod gc;
mod isolation;
mod mount;
pub mod registry;
mod security_filter;

use vrift_cas::CasStore;
use vrift_manifest::lmdb::{AssetTier, LmdbManifest};
use vrift_manifest::{Manifest, VnodeEntry};

/// Velo Rift™ - Content-Addressable Virtual Filesystem (Powered by VeloVFS)
#[derive(Parser)]
#[command(name = "vrift")]
#[command(version, about, long_about = None)]
struct Cli {
    /// TheSource™ storage root directory (global CAS)
    #[arg(
        long = "the-source-root",
        env = "VRIFT_CAS_ROOT",
        default_value = "~/.vrift/the_source"
    )]
    the_source_root: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Import files from a directory into the CAS (RFC-0039 Zero-Copy)
    Ingest {
        /// Directory to ingest
        #[arg(value_name = "DIR")]
        directory: PathBuf,

        /// Output manifest file path
        #[arg(short, long, default_value = "vrift.manifest")]
        output: PathBuf,

        /// Base path prefix in manifest (default: use directory name)
        #[arg(short, long)]
        prefix: Option<String>,

        /// Enable parallel file ingestion for better performance
        #[arg(long, default_value = "true")]
        parallel: bool,

        /// Number of parallel threads (default: min(cpu/2, 4), preserves system resources)
        #[arg(short = 'j', long)]
        threads: Option<usize>,

        /// Ingest mode: solid (hard_link, preserves source) or phantom (rename, moves to CAS)
        /// Default from config: storage.default_mode
        #[arg(long)]
        mode: Option<String>,

        /// Asset tier for solid mode: tier1 (immutable, symlink) or tier2 (mutable, keep original)
        /// Default from config: ingest.default_tier
        #[arg(long)]
        tier: Option<String>,

        /// Disable security filter (allow sensitive files like .env, *.key)
        #[arg(long)]
        no_security_filter: bool,

        /// Show files excluded by security filter
        #[arg(long)]
        show_excluded: bool,
    },

    /// Execute a command with VeloVFS virtualization
    Run {
        /// Manifest file to use
        #[arg(short, long, default_value = "vrift.manifest")]
        manifest: PathBuf,

        /// Command to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,

        /// Enable Linux namespace isolation
        #[arg(long)]
        isolate: bool,

        /// Optional base manifest for isolation
        #[arg(long)]
        base: Option<PathBuf>,

        /// Run via daemon (delegated execution)
        #[arg(long)]
        daemon: bool,
    },

    /// Display CAS statistics and session status
    Status {
        /// Also show manifest statistics if a manifest file is provided
        #[arg(short, long)]
        manifest: Option<PathBuf>,

        /// Show active session info
        #[arg(short = 's', long)]
        session: bool,

        /// Project directory (default: current directory)
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    /// Mount the manifest as a FUSE filesystem
    Mount(mount::MountArgs),

    /// Garbage Collect unreferenced blobs
    Gc(gc::GcArgs),

    /// Resolve dependencies from a velo.lock file
    Resolve {
        /// Lockfile path
        #[arg(short, long, default_value = "vrift.lock")]
        lockfile: PathBuf,
    },

    /// Daemon management
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Watch a directory for changes and auto-ingest
    Watch {
        /// Directory to watch
        #[arg(value_name = "DIR")]
        directory: PathBuf,

        /// Output manifest file path
        #[arg(short, long, default_value = "vrift.manifest")]
        output: PathBuf,
    },

    /// Activate Velo projection mode (RFC-0039)
    Active {
        /// Use Phantom mode (pure virtual projection)
        #[arg(long)]
        phantom: bool,

        /// Project directory (default: current directory)
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    /// Deactivate Velo projection
    Deactivate {
        /// Project directory (default: current directory)
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    /// Configuration management
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Check daemon status (ping)
    Status {
        /// Project directory (default: current directory)
        #[arg(short, long, value_name = "DIR")]
        directory: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Initialize default configuration file
    Init {
        /// Create global config (~/.vrift/config.toml)
        #[arg(long)]
        global: bool,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,
    },

    /// Show current configuration
    Show,

    /// Show configuration file path
    Path,

    /// Validate configuration file syntax
    Validate {
        /// Path to config file (default: auto-detect)
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    // Isolation check MUST happen before Tokio runtime starts (single-threaded requirement)
    if let Commands::Run {
        manifest,
        command,
        isolate,
        base,
        daemon: _, // daemon mode logic handled inside cmd_run if we get there
    } = &cli.command
    {
        if *isolate {
            // Validate inputs early
            // We can't use cmd_run directly because it might want async eventually,
            // but current cmd_run for isolation calls isolation::run_isolated which is sync.
            return isolation::run_isolated(
                command,
                manifest,
                &cli.the_source_root,
                base.as_deref(),
            );
        }
    }

    // Start Tokio Runtime for everything else
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Ingest {
            directory,
            output,
            prefix,
            parallel,
            threads,
            mode,
            tier,
            no_security_filter,
            show_excluded,
        } => {
            // Apply config defaults for mode and tier
            // Note: Extract values in block so MutexGuard is dropped before await
            let (mode, tier) = {
                let config = vrift_config::config();
                (
                    mode.unwrap_or_else(|| config.storage.default_mode.clone()),
                    tier.unwrap_or_else(|| config.ingest.default_tier.clone()),
                )
            };

            cmd_ingest(
                &cli.the_source_root,
                &directory,
                &output,
                prefix.as_deref(),
                parallel,
                threads,
                &mode,
                &tier,
                !no_security_filter,
                show_excluded,
            )
            .await
        }
        Commands::Run {
            manifest,
            command,
            isolate,
            base,
            daemon,
        } => cmd_run(
            &cli.the_source_root,
            &manifest,
            &command,
            isolate,
            base.as_deref(),
            daemon,
        ), // isolate is false here
        Commands::Status {
            manifest,
            session,
            directory,
        } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            cmd_status(&cli.the_source_root, manifest.as_deref(), session, &dir)
        }
        Commands::Mount(args) => mount::run(args),
        Commands::Gc(args) => gc::run(&cli.the_source_root, args).await,
        Commands::Resolve { lockfile } => cmd_resolve(&cli.the_source_root, &lockfile),
        Commands::Daemon { command } => match command {
            DaemonCommands::Status { directory } => {
                let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
                daemon::check_status(&dir).await
            }
        },
        Commands::Watch { directory, output } => {
            cmd_watch(&cli.the_source_root, &directory, &output).await
        }
        Commands::Active { phantom, directory } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            let mode = if phantom {
                active::ProjectionMode::Phantom
            } else {
                active::ProjectionMode::Solid
            };
            active::activate(&dir, mode)?;
            Ok(())
        }
        Commands::Deactivate { directory } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            active::deactivate(&dir)
        }
        Commands::Config { command } => cmd_config(command),
    }
}

/// Configuration management commands
fn cmd_config(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Init { global, force } => {
            let config_path = if global {
                vrift_config::Config::global_config_path()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?
            } else {
                PathBuf::from(".vrift/config.toml")
            };

            if config_path.exists() && !force {
                anyhow::bail!(
                    "Config file already exists: {}\nUse --force to overwrite",
                    config_path.display()
                );
            }

            // Create parent directory if needed
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Generate default config
            let default_config = vrift_config::Config::default_toml();
            std::fs::write(&config_path, default_config)?;

            println!("Created config file: {}", config_path.display());
            Ok(())
        }
        ConfigCommands::Show => {
            let config = vrift_config::config();
            let toml_str = toml::to_string_pretty(&*config)
                .map_err(|e| anyhow::anyhow!("Failed to serialize config: {}", e))?;
            println!("{}", toml_str);
            Ok(())
        }
        ConfigCommands::Path => {
            // Show which config files are being used
            if let Some(global_path) = vrift_config::Config::global_config_path() {
                let exists = global_path.exists();
                println!(
                    "Global: {} {}",
                    global_path.display(),
                    if exists { "[exists]" } else { "[not found]" }
                );
            }

            let project_path = PathBuf::from(".vrift/config.toml");
            let exists = project_path.exists();
            println!(
                "Project: {} {}",
                project_path.display(),
                if exists { "[exists]" } else { "[not found]" }
            );

            Ok(())
        }
        ConfigCommands::Validate { file } => {
            let config_path = if let Some(path) = file {
                path
            } else {
                // Auto-detect: prefer project config, fall back to global
                let project = PathBuf::from(".vrift/config.toml");
                if project.exists() {
                    project
                } else if let Some(global) = vrift_config::Config::global_config_path() {
                    if global.exists() {
                        global
                    } else {
                        anyhow::bail!(
                            "No config file found. Run 'vrift config init' to create one."
                        );
                    }
                } else {
                    anyhow::bail!("No config file found. Run 'vrift config init' to create one.");
                }
            };

            if !config_path.exists() {
                anyhow::bail!("Config file not found: {}", config_path.display());
            }

            println!("Validating: {}", config_path.display());
            let contents = std::fs::read_to_string(&config_path)?;

            match toml::from_str::<vrift_config::Config>(&contents) {
                Ok(config) => {
                    println!("✓ Syntax: Valid TOML");
                    println!("✓ Schema: All fields recognized");
                    println!();
                    println!("Summary:");
                    println!("  - Tier1 patterns: {}", config.tiers.tier1_patterns.len());
                    println!("  - Tier2 patterns: {}", config.tiers.tier2_patterns.len());
                    println!(
                        "  - Security patterns: {}",
                        config.security.exclude_patterns.len()
                    );
                    println!("  - Default mode: {}", config.storage.default_mode);
                    Ok(())
                }
                Err(e) => {
                    println!("✗ Validation failed!");
                    println!();
                    println!("Error: {}", e);
                    anyhow::bail!("Config validation failed");
                }
            }
        }
    }
}

fn cmd_resolve(cas_root: &Path, lockfile: &Path) -> Result<()> {
    if !lockfile.exists() {
        anyhow::bail!("Lockfile not found: {}", lockfile.display());
    }

    println!("Resolving lockfile: {}", lockfile.display());
    let lock = vrift_lock::VeloLock::load(lockfile)?;

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

/// Ingest a directory into the CAS using zero-copy operations (RFC-0039)
#[allow(clippy::too_many_arguments)]
async fn cmd_ingest(
    cas_root: &Path,
    directory: &Path,
    output: &Path,
    prefix: Option<&str>,
    _parallel: bool,
    threads: Option<usize>,
    mode: &str,
    tier: &str,
    security_filter_enabled: bool,
    show_excluded: bool,
) -> Result<()> {
    // Validate input directory
    if !directory.exists() {
        anyhow::bail!("Directory does not exist: {}", directory.display());
    }
    if !directory.is_dir() {
        anyhow::bail!("Not a directory: {}", directory.display());
    }

    // Parse mode and tier
    let is_phantom = mode.to_lowercase() == "phantom";
    let is_tier1 = tier.to_lowercase() == "tier1";
    let asset_tier = if is_tier1 {
        AssetTier::Tier1Immutable
    } else {
        AssetTier::Tier2Mutable
    };

    // Calculate thread count
    let thread_count = threads.unwrap_or_else(vrift_cas::default_thread_count);

    // Resolve CAS root (expand ~)
    let cas_root = if cas_root.starts_with("~") {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot resolve home directory"))?
            .join(cas_root.strip_prefix("~").unwrap_or(cas_root))
    } else {
        cas_root.to_path_buf()
    };
    std::fs::create_dir_all(&cas_root)?;

    // Initialize CAS store (still needed for legacy manifest + stats)
    let cas = CasStore::new(&cas_root)
        .with_context(|| format!("Failed to initialize CAS at {}", cas_root.display()))?;

    // Initialize LMDB manifest in project's .vrift directory
    let vrift_dir = directory.join(".vrift");
    std::fs::create_dir_all(&vrift_dir)?;
    let lmdb_manifest = LmdbManifest::open(vrift_dir.join("manifest.lmdb"))
        .with_context(|| "Failed to open LMDB manifest")?;

    // Mode banner
    let mode_str = if is_phantom {
        "Phantom (rename → CAS)"
    } else if is_tier1 {
        "Solid Tier-1 (hard_link + symlink)"
    } else {
        "Solid Tier-2 (hard_link, keep original)"
    };
    // Determine path prefix
    let base_prefix = prefix.unwrap_or_else(|| {
        directory
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("root")
    });

    // LMDB manifest initialized above (line 562)
    let mut files_ingested = 0u64;
    let mut bytes_ingested = 0u64;
    let mut unique_blobs = 0u64;
    let mut new_bytes = 0u64; // Track bytes from NEW blobs only
    let mut fallback_count = 0u64;

    // Print header
    println!("\n\u{26a1} VRift Ingest");
    println!("   Mode:    {} ", mode_str);
    println!("   CAS:     {}", cas_root.display());
    println!("   Threads: {}", thread_count);

    // Security filter status (RFC-0042)
    // Use config patterns when enabled, otherwise completely disabled
    let mut security_filter = if security_filter_enabled {
        security_filter::SecurityFilter::from_global_config()
    } else {
        security_filter::SecurityFilter::new(false)
    };
    if !security_filter_enabled {
        println!();
        println!("   \u{26a0}\u{fe0f}  SECURITY FILTER DISABLED (--no-security-filter)");
        println!("   \u{26a0}\u{fe0f}  Sensitive files (.env, *.key, etc.) WILL be ingested!");
    } else {
        println!("   \u{1f6e1}\u{fe0f}  Security: Filter ACTIVE");
    }

    // Collect entries with spinner feedback
    let scan_spinner = ProgressBar::new_spinner();
    scan_spinner.set_style(
        ProgressStyle::default_spinner()
            .template("   {spinner:.cyan} Scanning files... {msg}")
            .unwrap(),
    );
    scan_spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let mut entry_count = 0u64;
    let entries: Vec<_> = WalkDir::new(directory)
        .into_iter()
        .filter_map(|e| {
            if e.is_ok() {
                entry_count += 1;
                if entry_count.is_multiple_of(5000) {
                    scan_spinner.set_message(format!("{} entries", entry_count));
                }
            }
            e.ok()
        })
        .collect();
    scan_spinner.finish_and_clear();

    // Phase 1: Process directories and symlinks (must be serial for manifest order)
    // Also collect file paths for parallel processing
    let mut file_entries: Vec<(PathBuf, String, u64, u32)> = Vec::new(); // (path, manifest_path, mtime, mode)

    for entry in &entries {
        let path = entry.path();
        let relative = path.strip_prefix(directory).unwrap_or(path);

        // Skip .vrift directory
        if relative.starts_with(".vrift") {
            continue;
        }

        // Security filter check (RFC-0042)
        if let Some(reason) = security_filter.should_exclude(path) {
            security_filter.record_exclusion(path);
            if show_excluded {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    println!("   \u{1f6e1}\u{fe0f}  Excluded: {} ({})", name, reason);
                }
            }
            continue;
        }

        // Build manifest path
        let manifest_path = if relative.as_os_str().is_empty() {
            format!("/{}", base_prefix)
        } else {
            format!("/{}/{}", base_prefix, relative.display())
        };

        let metadata = fs::symlink_metadata(path)?;
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        if metadata.is_dir() {
            let vnode = VnodeEntry::new_directory(mtime, metadata.mode());
            lmdb_manifest.insert(&manifest_path, vnode, asset_tier);
        } else if metadata.is_file() {
            // Collect for parallel processing
            file_entries.push((path.to_path_buf(), manifest_path, mtime, metadata.mode()));
        } else if metadata.is_symlink() {
            let target = fs::read_link(path)?;
            let target_str = target
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Non-UTF8 symlink target: {}", path.display()))?;

            let content = target_str.as_bytes();
            let hash = CasStore::compute_hash(content);

            // Check if blob exists before storing (for accurate unique count)
            let blob_exists = cas.blob_path_for_hash(&hash).is_some();

            // Store symlink target string as a blob in CAS
            cas.store(content)?;

            // Count as unique if this blob was new
            if !blob_exists {
                unique_blobs += 1;
            }

            let vnode = VnodeEntry::new_symlink(hash, content.len() as u64, mtime);
            lmdb_manifest.insert(&manifest_path, vnode, asset_tier);
        }
    }

    // Phase 2: Parallel file ingest with progress bar
    let file_count = file_entries.len();
    let ingest_start = Instant::now();

    if file_count > 0 {
        let file_paths: Vec<PathBuf> = file_entries.iter().map(|(p, _, _, _)| p.clone()).collect();

        // Determine ingest mode
        let ingest_mode = if is_phantom {
            vrift_cas::IngestMode::Phantom
        } else if is_tier1 {
            vrift_cas::IngestMode::SolidTier1
        } else {
            vrift_cas::IngestMode::SolidTier2
        };

        // Create progress bar
        let pb = ProgressBar::new(file_count as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("   [{bar:40.cyan/blue}] {pos:>7}/{len:7} ({percent}%) • {msg}")
                .unwrap()
                .progress_chars("█▓░"),
        );
        pb.set_message("Processing...");

        // Shared counters for real-time stats
        let processed_bytes = Arc::new(AtomicU64::new(0));
        let new_blobs = Arc::new(AtomicU64::new(0));
        let pb_clone = pb.clone();
        let processed_bytes_clone = processed_bytes.clone();
        let new_blobs_clone = new_blobs.clone();
        let ingest_start_clone = ingest_start;
        let last_update = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Run parallel ingest with real-time progress callback
        let ingest_results = vrift_cas::parallel_ingest_with_progress(
            &file_paths,
            &cas_root,
            ingest_mode,
            Some(thread_count),
            move |result, idx| {
                // Update stats atomically
                if let Ok(ref r) = result {
                    processed_bytes_clone.fetch_add(r.size, Ordering::Relaxed);
                    if r.was_new {
                        new_blobs_clone.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Throttle: update every 100 files AND minimum 100ms since last update
                let count = idx + 1;
                if count % 100 == 0 {
                    let now_ms = ingest_start_clone.elapsed().as_millis() as u64;
                    let last_ms = last_update.load(Ordering::Relaxed);

                    // 100ms throttle
                    if now_ms >= last_ms + 100 {
                        last_update.store(now_ms, Ordering::Relaxed);

                        let elapsed = now_ms as f64 / 1000.0;
                        let rate = if elapsed > 0.0 {
                            count as f64 / elapsed
                        } else {
                            0.0
                        };
                        let total_bytes = processed_bytes_clone.load(Ordering::Relaxed);
                        let new_count = new_blobs_clone.load(Ordering::Relaxed);
                        let dedup_pct = if count > 0 {
                            100.0 * (1.0 - (new_count as f64 / count as f64))
                        } else {
                            0.0
                        };

                        // Only update position when also updating message (saves resources)
                        pb_clone.set_position(count as u64);
                        pb_clone.set_message(format!(
                            "{:.0} files/s • {} • {:.0}% dedup",
                            rate,
                            format_bytes(total_bytes),
                            dedup_pct
                        ));
                    }
                }
            },
        );

        // Phase 3: Update manifests from results (serial, but fast)
        for (i, result) in ingest_results.into_iter().enumerate() {
            let (_, ref manifest_path, mtime, mode) = file_entries[i];

            match result {
                Ok(ingest_result) => {
                    let vnode =
                        VnodeEntry::new_file(ingest_result.hash, ingest_result.size, mtime, mode);
                    lmdb_manifest.insert(manifest_path, vnode, asset_tier);

                    files_ingested += 1;
                    bytes_ingested += ingest_result.size;

                    if ingest_result.was_new {
                        unique_blobs += 1;
                        new_bytes += ingest_result.size; // Track actual new storage

                        // RFC-0043: Notify daemon of new blob for live index sync
                        let _ =
                            daemon::notify_blob(ingest_result.hash, ingest_result.size, directory)
                                .await;

                        // RFC-0039 §5.1.1: If Tier-1, request daemon to strengthen protection (chown + immutable)
                        if is_tier1 {
                            if let Some(blob_path) = cas.blob_path_for_hash(&ingest_result.hash) {
                                // Default daemon user is 'vrift'. If not set up, it will log warning and continue.
                                let _ = daemon::protect_file(
                                    blob_path,
                                    true,
                                    Some("vrift".to_string()),
                                    directory,
                                )
                                .await;
                            }
                        }
                    }
                }
                Err(e) => {
                    // Check for cross-device error (EXDEV)
                    if let vrift_cas::CasError::Io(ref io_err) = e {
                        if io_err.raw_os_error() == Some(libc::EXDEV) {
                            // Fallback to traditional copy
                            let path = &file_entries[i].0;
                            let content = fs::read(path).with_context(|| {
                                format!("Fallback read failed: {}", path.display())
                            })?;
                            let hash = CasStore::compute_hash(&content);
                            let size = content.len() as u64;
                            cas.store(&content)?;

                            let vnode = VnodeEntry::new_file(hash, size, mtime, mode);
                            lmdb_manifest.insert(manifest_path, vnode, asset_tier);

                            files_ingested += 1;
                            bytes_ingested += size;
                            fallback_count += 1;
                            pb.inc(1);
                            continue;
                        }
                    }
                    pb.abandon();
                    let path = &file_entries[i].0;
                    return Err(e).with_context(|| format!("Failed to ingest: {}", path.display()));
                }
            }
        }
        pb.finish_and_clear();
    }

    let ingest_elapsed = ingest_start.elapsed();

    // Commit LMDB manifest
    lmdb_manifest
        .commit()
        .with_context(|| "Failed to commit LMDB manifest")?;

    // Create and save legacy binary manifest for backward compatibility (FUSE, etc.)
    // RFC-0039 transitional support
    let mut legacy_manifest = vrift_manifest::Manifest::new();
    for (path, entry) in lmdb_manifest.iter()? {
        legacy_manifest.insert(&path, entry.vnode);
    }
    legacy_manifest
        .save(output)
        .with_context(|| format!("Failed to save binary manifest to {}", output.display()))?;

    // Auto-register in global manifest registry (RFC-0041)
    let output_abs = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    let directory_abs = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf());

    if let Ok(_lock) = registry::ManifestRegistry::acquire_lock() {
        if let Ok(mut reg) = registry::ManifestRegistry::load_or_create() {
            if let Ok(_uuid) = reg.register_manifest(&output_abs, &directory_abs) {
                let _ = reg.save();
            }
        }
    }

    let dedup_ratio = if files_ingested > 0 {
        100.0 * (1.0 - (unique_blobs as f64 / files_ingested as f64))
    } else {
        0.0
    };

    // Calculate speed metrics
    let elapsed_secs = ingest_elapsed.as_secs_f64();
    let files_per_sec = if elapsed_secs > 0.0 {
        files_ingested as f64 / elapsed_secs
    } else {
        0.0
    };
    let bytes_per_sec = if elapsed_secs > 0.0 {
        bytes_ingested as f64 / elapsed_secs
    } else {
        0.0
    };

    // Calculate REAL space savings: original size - new bytes added to CAS
    // Only was_new blobs add to CAS, duplicates are free!
    let saved_bytes = bytes_ingested.saturating_sub(new_bytes);
    let saved_pct = if bytes_ingested > 0 {
        100.0 * saved_bytes as f64 / bytes_ingested as f64
    } else {
        0.0
    };

    // ANSI color codes
    const GREEN: &str = "\x1b[32m";
    const CYAN: &str = "\x1b[36m";
    const YELLOW: &str = "\x1b[33m";
    const MAGENTA: &str = "\x1b[35m";
    const BOLD: &str = "\x1b[1m";
    const RESET: &str = "\x1b[0m";

    // Pretty output with colors
    println!();
    println!(
        "{}{}╔════════════════════════════════════════╗{}",
        BOLD, GREEN, RESET
    );
    println!(
        "{}{}║  ✅ VRift Complete in {:.2}s          ║{}",
        BOLD, GREEN, elapsed_secs, RESET
    );
    println!(
        "{}{}╚════════════════════════════════════════╝{}",
        BOLD, GREEN, RESET
    );
    println!();

    // Files -> Blobs conversion (the key metric)
    println!(
        "   {}{}📁 {} files → {} blobs{}",
        BOLD,
        CYAN,
        format_number(files_ingested),
        format_number(unique_blobs),
        RESET
    );

    // Dedup ratio - highlight if significant
    if dedup_ratio > 10.0 {
        println!(
            "   {}{}🔥 {:.1}% DEDUP{} - Content-Addressable Magic!",
            BOLD, MAGENTA, dedup_ratio, RESET
        );
    } else {
        println!("   {}📊 {:.1}% dedup{}", CYAN, dedup_ratio, RESET);
    }

    // Speed
    println!(
        "   {}⚡ {:.0} files/sec • {}/s{}",
        YELLOW,
        files_per_sec,
        format_bytes(bytes_per_sec as u64),
        RESET
    );

    // Space savings - prominent if significant
    if saved_bytes > 1024 * 1024 {
        // > 1MB
        println!(
            "   {}{}💾 SAVED {} ({:.1}% reduction){}",
            BOLD,
            GREEN,
            format_bytes(saved_bytes),
            saved_pct,
            RESET
        );
    } else if saved_bytes > 0 {
        println!(
            "   💾 Saved {} ({:.1}% reduction)",
            format_bytes(saved_bytes),
            saved_pct
        );
    }

    println!("   📄 Manifest: {}", output.display());
    if fallback_count > 0 {
        println!(
            "   {}⚠️  {} cross-device fallbacks{}",
            YELLOW, fallback_count, RESET
        );
    }

    // Security filter summary (RFC-0042)
    let excluded_count = security_filter.excluded_count();
    if excluded_count > 0 {
        println!(
            "   {}🛡️  {} sensitive files excluded{}",
            CYAN, excluded_count, RESET
        );
        if !show_excluded {
            println!("       (use --show-excluded for details)");
        }
    }
    println!();

    Ok(())
}

/// Execute a command with Velo VFS shim
fn cmd_run(
    cas_root: &Path,
    manifest: &Path,
    command: &[String],
    isolate: bool,
    base: Option<&Path>,
    daemon_mode: bool,
) -> Result<()> {
    if command.is_empty() {
        anyhow::bail!("No command specified");
    }

    // Delegation to daemon
    if daemon_mode {
        return tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let dir = std::env::current_dir().context("Failed to get current directory")?;
            rt.block_on(daemon::spawn_command(command, dir.clone(), &dir))
        });
    }

    if !manifest.exists() {
        anyhow::bail!("Manifest not found: {}", manifest.display());
    }

    // Handle isolation if requested
    if isolate {
        return isolation::run_isolated(command, manifest, cas_root, base);
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
    cmd.env("VRIFT_MANIFEST", &manifest_abs);
    cmd.env("VRIFT_CAS_ROOT", &cas_abs);

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

    // Enable debug output if VRIFT_DEBUG is set
    if std::env::var("VRIFT_DEBUG").is_ok() {
        cmd.env("VRIFT_DEBUG", "1");
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
                    p.join("libvrift_shim.dylib")
                }
                #[cfg(target_os = "linux")]
                {
                    p.join("libvrift_shim.so")
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    p.join("libvrift_shim.so")
                }
            }),
        // Installed location
        Some(PathBuf::from("/usr/local/lib/vrift/libvrift_shim.so")),
        #[cfg(target_os = "macos")]
        Some(PathBuf::from("/usr/local/lib/vrift/libvrift_shim.dylib")),
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
        Expected at: target/release/libvrift_shim.{}",
        if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        }
    );
}

/// Display CAS, manifest, and optionally session statistics
fn cmd_status(
    cas_root: &Path,
    manifest: Option<&Path>,
    show_session: bool,
    project_dir: &Path,
) -> Result<()> {
    println!("Velo Rift Status");
    println!("================");
    println!();

    // Session status (RFC-0039)
    if show_session {
        let vrift = active::VriftDir::new(project_dir);
        if vrift.has_session() {
            match vrift.load_session() {
                Ok(session) => {
                    let mode_icon = match session.mode {
                        active::ProjectionMode::Solid => "●",
                        active::ProjectionMode::Phantom => "◐",
                    };
                    let status = if session.active { "Active" } else { "Inactive" };
                    println!("Session: {} [{}] {}", mode_icon, session.mode, status);
                    println!("  Project:  {}", session.project_root.display());
                    println!("  Created:  {}", format_timestamp(session.created_at));
                    println!("  Platform: {}", session.abi_context.target_triple);
                    if let Some(ref rust) = session.abi_context.toolchain_version {
                        println!("  Rust:     {}", rust);
                    }
                    if let Some(ref py) = session.abi_context.python_version {
                        println!("  Python:   {}", py);
                    }
                    println!();
                }
                Err(e) => {
                    println!("Session: Error loading - {}", e);
                    println!();
                }
            }
        } else {
            println!("Session: None (run `vrift active` to start)");
            println!();
        }
    }

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

/// Format Unix timestamp as human-readable date
fn format_timestamp(epoch: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(epoch);
    // Simple formatting without chrono dependency
    let now = std::time::SystemTime::now();
    if let Ok(duration) = now.duration_since(dt) {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{} seconds ago", secs)
        } else if secs < 3600 {
            format!("{} minutes ago", secs / 60)
        } else if secs < 86400 {
            format!("{} hours ago", secs / 3600)
        } else {
            format!("{} days ago", secs / 86400)
        }
    } else {
        format!("epoch {}", epoch)
    }
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

/// Format number with comma separators (e.g., 1,234,567)
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().rev().collect();
    let chunks: Vec<String> = chars.chunks(3).map(|c| c.iter().collect()).collect();
    chunks.join(",").chars().rev().collect()
}

/// Watch a directory and auto-ingest on changes
async fn cmd_watch(cas_root: &Path, directory: &Path, output: &Path) -> Result<()> {
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    if !directory.exists() {
        anyhow::bail!("Directory to watch does not exist: {}", directory.display());
    }

    println!("Watching {} for changes...", directory.display());
    println!("Press Ctrl+C to stop.");

    // Initial ingest
    println!("\n[Initial Scan]");
    cmd_ingest(
        cas_root, directory, output, None, true, None, "solid", "tier2", true, false,
    )
    .await?;

    // Create a channel to receive the events.
    let (tx, rx) = channel();

    // Create a watcher object, delivering debounced events.
    // The notification back-end is selected based on the platform.
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(directory, RecursiveMode::Recursive)?;

    // Optimization: Debounce logic could be here (accumulate events over X ms)
    // For MVP, we just react to every event but maybe rate limit slightly?
    // Actually, `notify` handles some basic stuff, but repeated ingest is expensive if triggered too fast.
    // We'll implemented a simple loop that consumes events.

    let mut last_ingest = std::time::Instant::now();
    let debounce_duration = Duration::from_secs(1);

    loop {
        match rx.recv() {
            Ok(event_res) => {
                match event_res {
                    Ok(_event) => {
                        // Filter out unrelated events if needed, but for now we react to everything
                        // println!("Change detected: {:?}", event.paths);

                        // Simple debounce
                        if last_ingest.elapsed() > debounce_duration {
                            println!("\n[Change Detected] Re-ingesting...");
                            if let Err(e) = cmd_ingest(
                                cas_root, directory, output, None, true, None, "solid", "tier2",
                                true, false,
                            )
                            .await
                            {
                                eprintln!("Ingest failed: {}", e);
                            }
                            last_ingest = std::time::Instant::now();
                        }
                    }
                    Err(e) => println!("Watch error: {:?}", e),
                }
            }
            Err(e) => {
                println!("Watch channel error: {:?}", e);
                break;
            }
        }
    }

    Ok(())
}
