//! # vrift CLI
//!
//! Command-line interface for Velo Rift content-addressable filesystem.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use vrift_config::path::{normalize_for_ipc, normalize_or_original};

mod active;
mod daemon;
pub mod gc;
mod inception;
mod isolation;
mod mount;
pub mod registry;

use vrift_cas::CasStore;
use vrift_manifest::lmdb::LmdbManifest;

/// Velo Rift™ - Content-Addressable Virtual Filesystem (Powered by VeloVFS)
#[derive(Parser)]
#[command(name = "vrift")]
#[command(version, about, long_about = None)]
struct Cli {
    /// TheSource™ storage root directory (global CAS)
    #[arg(long = "the-source-root")]
    the_source_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Import files from a directory into the CAS (RFC-0039 Zero-Copy)
    Ingest {
        /// Directory to ingest
        #[arg(value_name = "DIR")]
        directory: PathBuf,

        /// Output manifest directory path
        #[arg(short, long, default_value = "vrift.manifest")]
        output: PathBuf,

        /// Base path prefix in manifest
        #[arg(short, long)]
        prefix: Option<String>,

        #[arg(long, default_value = "true")]
        parallel: bool,

        #[arg(short = 'j', long)]
        threads: Option<usize>,

        #[arg(long)]
        mode: Option<String>,

        #[arg(long)]
        tier: Option<String>,

        #[arg(long)]
        no_security_filter: bool,

        #[arg(long)]
        show_excluded: bool,

        #[arg(long)]
        force_hash: bool,
    },

    /// Execute a command with VeloVFS virtualization
    Run {
        #[arg(short, long, default_value = "vrift.manifest")]
        manifest: PathBuf,

        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,

        #[arg(long)]
        isolate: bool,

        #[arg(long)]
        base: Option<PathBuf>,

        #[arg(long)]
        daemon: bool,
    },

    /// Display CAS statistics and session status
    Status {
        #[arg(short, long)]
        manifest: Option<PathBuf>,

        #[arg(short = 's', long)]
        session: bool,

        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,

        #[arg(long)]
        inception: bool,
    },

    Mount(mount::MountArgs),
    Gc(gc::GcArgs),

    Resolve {
        #[arg(short, long, default_value = "vrift.lock")]
        lockfile: PathBuf,
    },

    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    Watch {
        #[arg(value_name = "DIR")]
        directory: PathBuf,

        #[arg(short, long, default_value = "vrift.manifest")]
        output: PathBuf,
    },

    Active {
        #[arg(long)]
        phantom: bool,
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    Deactivate {
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    Init {
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    Sync {
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    /// Inception layer - output shell env for eval
    Inception {
        #[arg(value_name = "DIR")]
        directory: Option<PathBuf>,
    },

    /// Wake - output shell env to exit VFS
    Wake,
}

#[derive(Subcommand)]
enum DaemonCommands {
    Status {
        #[arg(short, long, value_name = "DIR")]
        directory: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("VRIFT_LOG")
                .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let cli_cas_root_override = cli
        .the_source_root
        .as_ref()
        .map(|p| vrift_manifest::normalize_path(&p.to_string_lossy()));
    let cas_root =
        cli_cas_root_override
            .clone()
            .unwrap_or_else(|| match vrift_config::Config::load() {
                Ok(config) => config.storage.the_source,
                Err(_) => vrift_manifest::normalize_path(vrift_config::DEFAULT_CAS_ROOT),
            });

    if let Some(Commands::Run {
        manifest,
        command,
        isolate,
        base,
        daemon: _,
    }) = &cli.command
    {
        if *isolate {
            return isolation::run_isolated(command, manifest, &cas_root, base.as_deref());
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async_main(cli, cas_root))
}

async fn async_main(cli: Cli, cas_root: PathBuf) -> Result<()> {
    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            let dir = std::env::current_dir().unwrap();
            return inception::cmd_shell(&dir).await;
        }
    };

    match command {
        Commands::Ingest {
            directory,
            output,
            prefix,
            threads,
            mode,
            tier,
            force_hash,
            ..
        } => {
            let (mode_val, tier_val) = {
                let config = vrift_config::config();
                (
                    mode.unwrap_or_else(|| config.storage.default_mode.clone()),
                    tier.unwrap_or_else(|| config.ingest.default_tier.clone()),
                )
            };

            let is_phantom = mode_val.to_lowercase() == "phantom";
            let is_tier1 = tier_val.to_lowercase() == "tier1";
            let prefix_val = prefix.unwrap_or_else(|| "".to_string());

            let output_path = if output.to_string_lossy() == "vrift.manifest" {
                let vrift_dir = directory.join(".vrift");
                let _ = std::fs::create_dir_all(&vrift_dir);
                vrift_dir.join("manifest.lmdb")
            } else {
                output
            };

            let result = daemon::ingest_via_daemon(
                &directory,
                &output_path,
                threads,
                is_phantom,
                is_tier1,
                Some(prefix_val),
                Some(&cas_root),
                force_hash,
            )
            .await?;

            println!(
                "\n✅ Ingest Complete: {} files -> {} blobs",
                result.files, result.blobs
            );
            println!("📄 Manifest: {}", result.manifest_path);

            let mut registry = registry::ManifestRegistry::load_or_create()?;
            let _ = registry.register_manifest(Path::new(&result.manifest_path), &directory);
            let _ = registry.save();
            Ok(())
        }
        Commands::Run {
            manifest,
            command,
            isolate,
            base,
            daemon,
        } => cmd_run(
            &cas_root,
            &manifest,
            &command,
            isolate,
            base.as_deref(),
            daemon,
        ),
        Commands::Status {
            manifest,
            session,
            directory,
            inception: _,
        } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            cmd_status(&cas_root, manifest.as_deref(), session, &dir)
        }
        Commands::Mount(args) => mount::run(args, &cas_root),
        Commands::Gc(args) => gc::run(&cas_root, args).await,
        Commands::Resolve { lockfile } => {
            println!("Resolving lockfile: {}", lockfile.display());
            let lock = vrift_lock::VeloLock::load(&lockfile)?;
            let cas = CasStore::new(&cas_root)?;
            let mut resolved = 0;
            for pkg in lock.packages.values() {
                if let Some(hash_str) = pkg.source_tree.strip_prefix("tree:") {
                    if let Some(hash) = CasStore::hex_to_hash(hash_str) {
                        if cas.exists(&hash) {
                            resolved += 1;
                        }
                    }
                }
            }
            println!("Result: {} resolved", resolved);
            Ok(())
        }
        Commands::Daemon { command } => match command {
            DaemonCommands::Status { directory } => {
                let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
                daemon::check_status(&dir).await
            }
        },
        Commands::Watch { directory, .. } => {
            println!("Watching {}...", directory.display());
            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            vrift_vdird::watch::spawn_watch_task(directory.clone(), tx);
            while let Some(event) = rx.recv().await {
                println!("Change Detected: {:?}", event);
            }
            Ok(())
        }
        Commands::Active { phantom, directory } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            let mode = if phantom {
                active::ProjectionMode::Phantom
            } else {
                active::ProjectionMode::Solid
            };
            active::activate(&dir, mode).map(|_| ())
        }
        Commands::Deactivate { directory } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            active::deactivate(&dir)
        }
        Commands::Init { directory } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            cmd_init(&dir).await
        }
        Commands::Sync { .. } => Ok(()),
        Commands::Inception { directory } => {
            let dir = directory.unwrap_or_else(|| std::env::current_dir().unwrap());
            inception::cmd_inception(&dir).await
        }
        Commands::Wake => inception::cmd_wake(),
    }
}

async fn cmd_init(directory: &Path) -> Result<()> {
    let vrift_dir = directory.join(".vrift");
    fs::create_dir_all(&vrift_dir)?;
    fs::create_dir_all(vrift_dir.join("bin"))?;
    fs::create_dir_all(vrift_dir.join("locks"))?;
    fs::create_dir_all(vrift_dir.join("sockets"))?;
    let _ = LmdbManifest::open(vrift_dir.join("manifest.lmdb"))?;
    println!("Initialized Velo Rift in {}", directory.display());
    Ok(())
}

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
    if daemon_mode {
        use anyhow::Context;
        return tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let dir = std::env::current_dir().context("cwd")?;
            rt.block_on(daemon::spawn_command(command, dir.clone(), &dir))
        });
    }
    if isolate {
        return isolation::run_isolated(command, manifest, cas_root, base);
    }
    let _shim_path = find_shim_library()?;
    let mut cmd = std::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    let manifest_normalized = normalize_for_ipc(manifest)?;
    let cas_normalized = normalize_or_original(cas_root);

    // Set Velo environment variables
    cmd.env("VRIFT_MANIFEST", &manifest_normalized);
    cmd.env("VR_THE_SOURCE", &cas_normalized);

    // RFC-0050: Auto-derive VFS prefix from manifest path if not set
    let manifest_str = manifest_normalized.to_string_lossy();
    let root_path = if let Some(idx) = manifest_str.find("/.vrift/") {
        &manifest_str[..idx]
    } else if let Some(stripped) = manifest_str.strip_suffix("/.vrift") {
        stripped
    } else if let Some(idx) = manifest_str.rfind('/') {
        &manifest_str[..idx]
    } else {
        manifest_str.as_ref()
    };
    cmd.env("VRIFT_VFS_PREFIX", root_path);

    // Set platform-specific library preload
    #[cfg(target_os = "macos")]
    {
        cmd.env("DYLD_INSERT_LIBRARIES", &_shim_path);
        cmd.env("DYLD_FORCE_FLAT_NAMESPACE", "1");
    }
    #[cfg(target_os = "linux")]
    {
        cmd.env("LD_PRELOAD", &_shim_path);
    }
    // Enable debug/profile output if set
    if std::env::var("VRIFT_DEBUG").is_ok() {
        cmd.env("VRIFT_DEBUG", "1");
    }
    if std::env::var("VRIFT_PROFILE").is_ok() {
        cmd.env("VRIFT_PROFILE", "1");
    }
    if let Ok(log) = std::env::var("VRIFT_LOG_LEVEL") {
        cmd.env("VRIFT_LOG_LEVEL", log);
    }

    use anyhow::Context;
    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute: {}", command[0]))?;
    std::process::exit(status.code().unwrap_or(1));
}

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
                    p.join("libvrift_inception_layer.dylib")
                }
                #[cfg(target_os = "linux")]
                {
                    p.join("libvrift_inception_layer.so")
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    p.join("libvrift_inception_layer.so")
                }
            }),
        // Also check target/release relative to CWD if not in EXE dir
        Some(PathBuf::from("target/release").join(if cfg!(target_os = "macos") {
            "libvrift_inception_layer.dylib"
        } else {
            "libvrift_inception_layer.so"
        })),
        // Installed location (standard Linux/FHS)
        Some(PathBuf::from(
            "/usr/local/lib/vrift/libvrift_inception_layer.so",
        )),
        #[cfg(target_os = "macos")]
        Some(PathBuf::from(
            "/usr/local/lib/vrift/libvrift_inception_layer.dylib",
        )),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Provide helpful error message
    anyhow::bail!(
        "Could not find vrift-inception-layer library. \n\
        Build with: cargo build -p vrift-inception-layer --release\n\
        Expected at: target/release/libvrift_inception_layer.{}",
        if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        }
    );
}

fn cmd_status(
    cas_root: &Path,
    manifest: Option<&Path>,
    _session: bool,
    _project_dir: &Path,
) -> Result<()> {
    if cas_root.exists() {
        let cas = CasStore::new(cas_root)?;
        println!("CAS: {} Unique blobs", cas.stats()?.blob_count);
    }
    if let Some(manifest_path) = manifest {
        let m = LmdbManifest::open(manifest_path)?;
        println!("Manifest: {} files", m.stats()?.file_count);
    }
    Ok(())
}
