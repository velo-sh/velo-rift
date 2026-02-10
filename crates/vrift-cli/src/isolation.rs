//! Isolation module for Velo Rift
//!
//! Handles Linux namespace creation and setup for isolated execution.

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
use std::path::Path;

#[cfg(target_os = "linux")]
use nix::sched::{unshare, CloneFlags};

/// Run a command in an isolated environment
#[allow(unused_variables)]
pub fn run_isolated(
    command: &[String],
    manifest_path: &Path,
    cas_root: &Path,
    base_manifest_path: Option<&Path>,
) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Isolation is not supported on this operating system.");
    }

    #[cfg(target_os = "linux")]
    {
        run_isolated_linux(command, manifest_path, cas_root, base_manifest_path)
    }
}

#[cfg(target_os = "linux")]
fn run_isolated_linux(
    command: &[String],
    manifest: &Path,
    cas_root: &Path,
    base_manifest: Option<&Path>,
) -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    println!("🔒 Setting up isolated container...");

    // Capture current UID/GID before unsharing
    let uid = nix::unistd::getuid();
    let gid = nix::unistd::getgid();

    let flags = CloneFlags::CLONE_NEWNS
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWUTS
        | CloneFlags::CLONE_NEWUSER;

    unshare(flags).context("Failed to unshare namespaces")?;

    // Map current user to root (0) inside the namespace
    write_id_map("/proc/self/uid_map", uid.as_raw(), 0, 1)?;
    write_id_map("/proc/self/setgroups", 0, 0, 0)?;
    write_id_map("/proc/self/gid_map", gid.as_raw(), 0, 1)?;

    // Setup Mounts
    setup_mounts(manifest, cas_root, base_manifest)?;

    // 4. Exec Command
    let err = Command::new(&command[0])
        .args(&command[1..])
        .env("VRIFT_ISOLATED", "1")
        .exec();

    anyhow::bail!("Failed to exec: {}", err);
}

#[cfg(target_os = "linux")]
fn write_id_map(path: &str, real_id: u32, inside_id: u32, count: u32) -> Result<()> {
    if path.ends_with("setgroups") {
        std::fs::write(path, "deny")
            .with_context(|| format!("Failed to write deny to {}", path))?;
        return Ok(());
    }

    let content = format!("{} {} {}", inside_id, real_id, count);
    std::fs::write(path, content).with_context(|| format!("Failed to write id map to {}", path))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn setup_mounts(
    manifest_path: &Path,
    cas_root: &Path,
    base_manifest_path: Option<&Path>,
) -> Result<()> {
    use std::fs;

    use vrift_cas::CasStore;
    use vrift_manifest::LmdbManifest;
    use vrift_runtime::{LinkFarm, NamespaceManager, OverlayManager};

    let run_dir = tempfile::Builder::new().prefix("velo-run-").tempdir()?;
    let run_path_persistent = run_dir.keep();

    println!(
        "   [Isolation] Runtime State: {}",
        run_path_persistent.display()
    );

    let lower_dir = run_path_persistent.join("lower");
    let upper_dir = run_path_persistent.join("upper");
    let work_dir = run_path_persistent.join("work");
    let merged_dir = run_path_persistent.join("merged");

    // 1. Load Manifests & CAS
    let mut manifests = Vec::new();

    // RFC-0039: Only support LMDB manifests for isolation
    if let Some(base_path) = base_manifest_path {
        let base_manifest = LmdbManifest::open(base_path)
            .with_context(|| format!("Failed to load base manifest: {}", base_path.display()))?;
        manifests.push(base_manifest);
        println!("   [Isolation] Using base image: {}", base_path.display());
    }

    let manifest = LmdbManifest::open(manifest_path)
        .with_context(|| format!("Failed to load manifest: {}", manifest_path.display()))?;
    manifests.push(manifest);

    let cas = CasStore::new(cas_root)
        .with_context(|| format!("Failed to open CAS: {}", cas_root.display()))?;

    // 2. Populate LowerDir (Link Farm)
    println!("   [Isolation] Populating LowerDir...");
    let link_farm = LinkFarm::new(cas);
    link_farm
        .populate(&manifests, &lower_dir)
        .context("Failed to populate Link Farm")?;

    // 3. Mount OverlayFS
    println!("   [Isolation] Mounting OverlayFS...");
    let overlay = OverlayManager::new(lower_dir.clone(), upper_dir, work_dir, merged_dir.clone());

    let mut use_overlay = true;
    let root_for_pivot = if let Err(e) = overlay.mount() {
        println!("   [Isolation] ⚠️ OverlayFS mount failed: {}", e);
        println!("   [Isolation] ⚠️ Falling back to Read-Only Bind Mount (No CoW).");
        use_overlay = false;

        use nix::mount::{mount, MsFlags};
        mount(
            Some(&lower_dir),
            &lower_dir,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .context("Failed to bind mount lower_dir for fallback")?;

        lower_dir
    } else {
        merged_dir
    };

    // 4. Bind-mount CAS root into jail
    let cas_in_jail = root_for_pivot.join(cas_root.strip_prefix("/").unwrap_or(cas_root));
    fs::create_dir_all(&cas_in_jail).context("Failed to create CAS mountpoint in jail")?;

    use nix::mount::{mount, MsFlags};
    mount(
        Some(cas_root),
        &cas_in_jail,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_RDONLY,
        None::<&str>,
    )
    .with_context(|| {
        format!(
            "Failed to bind mount CAS into jail: {} -> {}",
            cas_root.display(),
            cas_in_jail.display()
        )
    })?;

    // 5. Pivot Root
    let old_root_path = root_for_pivot.join(".old_root");
    fs::create_dir_all(&old_root_path).context("Failed to create .old_root")?;

    std::env::set_current_dir(&root_for_pivot).context("Failed to chdir to new root")?;

    println!("   [Isolation] Pivot Root -> . (old=.old_root)");
    NamespaceManager::pivot_root(Path::new("."), Path::new(".old_root"))
        .context("Failed to pivot_root")?;

    if !use_overlay {
        use nix::mount::{mount, MsFlags};
        println!("   [Isolation] Remounting root Read-Only...");
        mount(
            Some(""),
            Path::new("/"),
            None::<&str>,
            MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY,
            None::<&str>,
        )
        .context("Failed to remount root Read-Only")?;
    }

    println!("   [Isolation] Mounting /proc, /sys, /dev...");
    let new_root = Path::new("/");
    NamespaceManager::mount_pseudo_fs(new_root).context("Failed to mount pseudo-filesystems")?;

    Ok(())
}
