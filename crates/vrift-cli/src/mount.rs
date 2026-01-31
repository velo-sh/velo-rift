use anyhow::{Context, Result};
use clap::Args;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(feature = "fuse")]
use vrift_cas::CasStore;
#[cfg(feature = "fuse")]
use vrift_manifest::Manifest;

#[derive(Args, Debug)]
pub struct MountArgs {
    /// Manifest file to mount
    #[arg(short, long, default_value = "vrift.manifest")]
    manifest: PathBuf,

    /// Mount point directory
    #[arg(value_name = "MOUNTPOINT")]
    mountpoint: PathBuf,
}

/// Execute the mount command
pub fn run(args: MountArgs) -> Result<()> {
    let cas_root =
        std::env::var("VR_THE_SOURCE").unwrap_or_else(|_| "~/.vrift/the_source".to_string());
    let cas_root = Path::new(&cas_root);
    let manifest_path = &args.manifest;
    let mountpoint = &args.mountpoint;
    if !manifest_path.exists() {
        anyhow::bail!("Manifest not found: {}", manifest_path.display());
    }

    if !cas_root.exists() {
        anyhow::bail!("CAS root not found: {}", cas_root.display());
    }

    // Ensure mountpoint exists
    if !mountpoint.exists() {
        fs::create_dir_all(mountpoint)
            .with_context(|| format!("Failed to create mountpoint: {}", mountpoint.display()))?;
    }

    tracing::info!("Mounting Velo Rift™...");
    tracing::info!("  Manifest:   {}", manifest_path.display());
    tracing::info!("  CAS:        {}", cas_root.display());
    tracing::info!("  Mountpoint: {}", mountpoint.display());
    tracing::info!("  Mode:       Read-Only");

    #[cfg(feature = "fuse")]
    {
        let cas = CasStore::new(cas_root)?;
        let manifest = Manifest::load(manifest_path)?;
        let fs = vrift_fuse::VeloFs::new(&manifest, cas);

        // This will block until unmounted
        fs.mount(mountpoint)?;
    }

    #[cfg(not(feature = "fuse"))]
    {
        tracing::warn!("FUSE support disabled. Recompile with --features fuse to enable.");
        tracing::warn!("    cargo build -p velo-cli --features fuse");
    }

    Ok(())
}
