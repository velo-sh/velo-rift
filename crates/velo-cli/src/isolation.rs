//! Isolation module for Velo Rift
//!
//! Handles Linux namespace creation and setup for isolated execution.

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
use std::path::Path;

#[cfg(target_os = "linux")]
use nix::{
    // mount::{mount, MsFlags},
    sched::{unshare, CloneFlags},
    // unistd::{chdir, pivot_root},
};

/// Run a command in an isolated environment
pub fn run_isolated(
    command: &[String],
    manifest_path: &Path,
    cas_root: &Path,
    base_manifest_path: Option<&Path>,
) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    #[allow(unused_variables)]
    {
        // On non-Linux platforms, just warn and fall back to normal execution
        eprintln!("⚠️  Isolation is only fully supported on Linux.");
        eprintln!("   Running with standard LD_PRELOAD process isolation instead.");

        // This function is called before the command execution in main.rs,
        // so returning Ok here means "proceed with normal execution".
        // However, the caller in main.rs needs to know whether to execute the command itself
        // or if we already handled it.
        // For this design, let's assume this function handles EVERYTHING if it returns Ok,
        // but for non-Linux fallback, we want to return a specific error or use a different pattern.

        // Better approach: main.rs calls this, if it is not linux, we bail out or warn.
        // But since we want fallback, let's return a special "NotSupported" error
        // or let main.rs handle the platform check.

        // Simple fallback:
        // We do nothing here, and main.rs proceeds to normal execution?
        // No, run_isolated is supposed to TAKE OVER execution.

        // Let's decided: On macOS, we error out if --isolate is requested explicitly?
        // Or we just warn.
        // Given the prompt "Review Required: Isolation features will only work on Linux",
        // let's error out if explicit isolation is requested on non-Linux.

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

    // 1. Unshare Namespaces
    // We need NEWNS (Mount), NEWPID (Process), NEWIPC, NEWUTS.
    // NEWUSER is complex (mapping UIDs), skipping for MVP unless root.
    // NEWNET requires network setup, skipping for MVP (host net).
    // We include CLONE_NEWUSER to allow rootless execution.
    let flags = CloneFlags::CLONE_NEWNS
        // | CloneFlags::CLONE_NEWPID // Disabled for CI stability (Docker-on-macOS QEMU limits)
        | CloneFlags::CLONE_NEWIPC
        | CloneFlags::CLONE_NEWUTS
        | CloneFlags::CLONE_NEWUSER;

    unshare(flags).context("Failed to unshare namespaces")?;

    // 2. Setup ID Mappings (User Namespace)
    // Map current user to root (0) inside the namespace
    // We must do this *before* any mount operations, as we need capabilities.
    write_id_map("/proc/self/uid_map", uid.as_raw(), 0, 1)?;
    write_id_map("/proc/self/setgroups", 0, 0, 0)?; // "deny" for setgroups if possible, or just ignore for single user
    write_id_map("/proc/self/gid_map", gid.as_raw(), 0, 1)?;

    // 3. Fork?
    // unshare(CLONE_NEWPID) only affects *children*. We need to fork to become pid 1 in new ns.
    // Simplified: We assume current process is now the setup process,
    // and we exec into the target. But for PID ns to work, we need a child.

    // Nix fork is unsafe. We might use std::process::Command to spawn ourselves as a child helper?
    // Or just rely on the fact that we will exec.

    // Correct pattern for unshare(NEWPID):
    // 1. unshare()
    // 2. fork()
    // 3. parent waits, child is PID 1 in new NS.

    // For MVP, handling fork in Rust safely is tricky without external crate help like `unshare` or `standard container runtimes`.
    // However, we can try a simpler approach if we don't strictly need to be PID 1 *yet*
    // or if we just want Mount isolation.

    // Let's stick to MOUNT isolation for now as it's the critical part for file system view.
    // PID isolation is good but harder.

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
    // Special case for setgroups: usually we write "deny"
    if path.ends_with("setgroups") {
         std::fs::write(path, "deny")
            .with_context(|| format!("Failed to write deny to {}", path))?;
         return Ok(());
    }

    let content = format!("{} {} {}", inside_id, real_id, count);
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write id map to {}", path))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn setup_mounts(
    manifest_path: &Path,
    cas_root: &Path,
    base_manifest_path: Option<&Path>,
) -> Result<()> {
    use std::fs;

    use velo_cas::CasStore;
    use velo_manifest::Manifest;
    use velo_runtime::{LinkFarm, NamespaceManager, OverlayManager};

    // We need a persistent temp directory that survives this function
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

    // Load base manifest first if provided
    if let Some(base_path) = base_manifest_path {
        let base_manifest = Manifest::load(base_path)
            .with_context(|| format!("Failed to load base manifest: {}", base_path.display()))?;
        manifests.push(base_manifest);
        println!("   [Isolation] Using base image: {}", base_path.display());
    }

    let manifest = Manifest::load(manifest_path)
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
    
    // Fallback logic variables
    let mut use_overlay = true;
    let root_for_pivot = if let Err(e) = overlay.mount() {
        println!("   [Isolation] ⚠️ OverlayFS mount failed: {}", e);
        println!("   [Isolation] ⚠️ Falling back to Read-Only Bind Mount (No CoW).");
        
        // Fallback: Use lower_dir as root, but make it Read-Only to protect CAS hardlinks.
        use_overlay = false;
        
        // To pivot_root, 'new_root' must be a mount point.
        // Bind mount lower_dir to itself.
        // We use nix::mount::mount directly.
        use nix::mount::{mount, MsFlags};
        
        mount(
            Some(&lower_dir),
            &lower_dir,
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        ).context("Failed to bind mount lower_dir for fallback")?;
        
        // We will maintain it RW for a moment to create .old_root, then remount RO?
        // Actually, we can create .old_root before remounting RO logic if we want, 
        // but typically we can just create it now since we are still outside the namespace restriction effectively 
        // (or rather we are root inside it).
        
        lower_dir 
    } else {
        merged_dir
    };

    // 4. Pivot Root
    let old_root_path = root_for_pivot.join(".old_root");
    fs::create_dir_all(&old_root_path).context("Failed to create .old_root")?;

    std::env::set_current_dir(&root_for_pivot).context("Failed to chdir to new root")?;

    println!("   [Isolation] Pivot Root -> . (old=.old_root)");
    NamespaceManager::pivot_root(Path::new("."), Path::new(".old_root"))
        .context("Failed to pivot_root")?;
        
    // If fallback, remount root as Read-Only NOW
    if !use_overlay {
        use nix::mount::{mount, MsFlags};
         println!("   [Isolation] Remounting root Read-Only...");
         mount(
            Some(""),
            Path::new("/"),
            None::<&str>,
            MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY,
            None::<&str>,
        ).context("Failed to remount root Read-Only")?;
    }

    // 5. Mount Pseudo-FS (/proc, /sys, /dev) in the new root
    // Now that we are pivoted, "/" is the merged dir.
    println!("   [Isolation] Mounting /proc, /sys, /dev...");
    let new_root = Path::new("/");
    NamespaceManager::mount_pseudo_fs(new_root).context("Failed to mount pseudo-filesystems")?;

    Ok(())
}
