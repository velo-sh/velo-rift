//! # Inception Commands
//!
//! Movie-inspired VFS activation system - "Enter the Dream"
//!
//! - `vrift inception` - Enter VFS layer (outputs shell script for eval)
//! - `vrift wake` - Exit VFS layer
//! - `vrift hook <shell>` - Generate shell hook for auto-inception

#![allow(clippy::print_literal)] // Styled output intentionally uses format strings

use std::env;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use console::{style, Emoji};
use indicatif::{ProgressBar, ProgressStyle};
use vrift_config::path::normalize_for_ipc;

// Emojis for theatrical effect
static TOTEM_SPIN: Emoji<'_, '_> = Emoji("🌀", "*");
static BELL: Emoji<'_, '_> = Emoji("🔔", "*");
static CHECK: Emoji<'_, '_> = Emoji("✔ ", "[ok] ");
static WARN: Emoji<'_, '_> = Emoji("⚠️  ", "! ");

// Box drawing characters
const BOX_TL: &str = "╭";
const BOX_TR: &str = "╮";
const BOX_BL: &str = "╰";
const BOX_BR: &str = "╯";
const BOX_H: &str = "─";
const BOX_V: &str = "│";

// ============================================================================
// Main Entry Point: vrift shell (or just `vrift` with no args)
// ============================================================================

/// Enter VFS mode by spawning a new interactive subshell
///
/// This is the primary UX - no eval needed!
pub async fn cmd_shell(project_dir: &Path) -> Result<()> {
    use std::process::Command;

    // Phase 1.2: Connect to daemon FIRST — this triggers RegisterWorkspace which
    // spawns vDird and creates vdir.mmap. Must happen before preflight checks.
    let daemon_conn = match crate::daemon::connect_to_daemon(project_dir).await {
        Ok(conn) => Some(conn),
        Err(e) => {
            eprintln!("{} Daemon connection warning: {}", style("⚠").yellow(), e);
            None
        }
    };

    // =========================================================================
    // Preflight Check: Fail-fast, fail-early (RFC: Inception Preflight)
    // Now runs AFTER daemon connection so vdir.mmap exists.
    // =========================================================================
    let preflight = crate::preflight::run_preflight(project_dir);
    if !preflight.can_activate {
        crate::preflight::print_preflight_errors(&preflight);
        std::process::exit(1);
    }

    // RFC-0052: Manage session persistence
    let _session = crate::active::activate(project_dir, crate::active::ProjectionMode::Solid)?;

    let vrift_dir = project_dir.join(".vrift");
    let project_root = normalize_for_ipc(project_dir).context("resolve project path")?;
    let project_root_str = project_root.to_string_lossy();
    let project_name = project_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    // Check if already in inception
    if env::var("VRIFT_INCEPTION").is_ok() {
        eprintln!(
            "{} Already in inception mode. Type 'exit' to wake up.",
            WARN
        );
        return Ok(());
    }

    // Ensure wrappers exist
    ensure_wrappers(&vrift_dir)?;

    // Find the shim library
    let inception_path = find_inception_library(&project_root)?;

    // Get VFS stats
    let (file_count, cas_size) = get_vfs_stats(&vrift_dir);

    // Show animation
    show_inception_animation();

    // Print inception box
    eprintln!();
    eprintln!("{}{}{}", BOX_TL, BOX_H.repeat(35), BOX_TR);
    eprintln!(
        "{} {} INCEPTION                       {}",
        BOX_V, TOTEM_SPIN, BOX_V
    );
    eprintln!("{}                                    {}", BOX_V, BOX_V);
    eprintln!(
        "{}    Project: {:<23} {}",
        BOX_V,
        truncate_str(&project_name, 23),
        BOX_V
    );
    eprintln!(
        "{}    VFS: {} files │ {:<15} {}",
        BOX_V, file_count, cas_size, BOX_V
    );
    eprintln!("{}{}{}", BOX_BL, BOX_H.repeat(35), BOX_BR);
    eprintln!();
    eprintln!("{}", style("Type 'exit' or Ctrl+D to wake up").dim());
    eprintln!();

    // Detect user's shell
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

    // Build new PATH with wrappers
    let current_path = env::var("PATH").unwrap_or_default();
    let new_path = format!("{}/.vrift/bin:{}", project_root_str, current_path);

    // Derive shim environment from Config SSOT
    let cfg = vrift_config::Config::load_for_project(&project_root).unwrap_or_else(|e| {
        eprintln!("Warning: Config load failed: {}. Using defaults.", e);
        vrift_config::Config::default()
    });
    let shim_env = cfg.shim_env();

    // Spawn subshell with VFS environment
    let mut cmd = Command::new(&shell);
    cmd.current_dir(&project_root)
        .env("VRIFT_INCEPTION", "1")
        .env("PATH", new_path);

    // Phase 1.2: Inject vDird socket + mmap paths for zero-RPC inception init
    if let Some(ref conn) = daemon_conn {
        if !conn.vdird_socket.is_empty() {
            cmd.env("VRIFT_VDIRD_SOCKET", &conn.vdird_socket);
        }
        if !conn.vdir_mmap_path.is_empty() {
            cmd.env("VRIFT_VDIR_MMAP", &conn.vdir_mmap_path);
        }
    }

    // Apply all SSOT-derived env vars
    for (key, value) in &shim_env {
        cmd.env(key, value);
    }

    #[cfg(target_os = "macos")]
    {
        cmd.env(
            "DYLD_INSERT_LIBRARIES",
            inception_path.to_string_lossy().as_ref(),
        )
        .env("DYLD_FORCE_FLAT_NAMESPACE", "1");
    }
    #[cfg(target_os = "linux")]
    {
        cmd.env("LD_PRELOAD", inception_path.to_string_lossy().as_ref());
    }

    let status = cmd
        .env("PS1", format!("(vrift {}) $PS1", TOTEM_SPIN))
        .status()?;

    // Wake up message
    eprintln!();
    eprintln!("{}{}{}", BOX_TL, BOX_H.repeat(35), BOX_TR);
    eprintln!(
        "{} {} WAKE                            {}",
        BOX_V, BELL, BOX_V
    );
    eprintln!("{}{}{}", BOX_BL, BOX_H.repeat(35), BOX_BR);
    eprintln!();

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// Generate shell script for `eval "$(vrift inception)"`
pub async fn cmd_inception(project_dir: &Path) -> Result<()> {
    // Phase 1.2: Connect to daemon FIRST — this triggers RegisterWorkspace which
    // spawns vDird and creates vdir.mmap. Must happen before preflight checks.
    let daemon_conn = match crate::daemon::connect_to_daemon(project_dir).await {
        Ok(conn) => Some(conn),
        Err(e) => {
            eprintln!("{} Daemon connection warning: {}", style("⚠").yellow(), e);
            None
        }
    };

    // =========================================================================
    // Preflight Check: Fail-fast, fail-early (RFC: Inception Preflight)
    // Now runs AFTER daemon connection so vdir.mmap exists.
    // =========================================================================
    let preflight = crate::preflight::run_preflight(project_dir);
    if !preflight.can_activate {
        crate::preflight::print_preflight_errors(&preflight);
        std::process::exit(1);
    }

    // RFC-0052: Manage session persistence
    let _session = crate::active::activate(project_dir, crate::active::ProjectionMode::Solid)?;

    let vrift_dir = project_dir.join(".vrift");

    // Check if already in inception
    if env::var("VRIFT_INCEPTION").is_ok() {
        eprintln!(
            "{} {}",
            WARN,
            style("Already in inception mode. Use 'vrift wake' to exit first.").yellow()
        );
        std::process::exit(0);
    }

    let project_root = normalize_for_ipc(project_dir).context("Failed to resolve project path")?;
    let project_root_str = project_root.to_string_lossy();

    // Find the shim library
    let inception_path = find_inception_library(&project_root)?;

    // Ensure wrappers exist in .vrift/bin/
    ensure_wrappers(&vrift_dir)?;

    // Get VFS stats
    let (file_count, cas_size) = get_vfs_stats(&vrift_dir);

    // Theatrical progress (to stderr so it doesn't interfere with eval)
    show_inception_animation();

    // Get the path to the vrift binary itself
    let vrift_bin_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();

    // Derive shim environment from Config SSOT
    let cfg = vrift_config::Config::load_for_project(&project_root).unwrap_or_else(|e| {
        eprintln!("Warning: Config load failed: {}. Using defaults.", e);
        vrift_config::Config::default()
    });
    let shim_env = cfg.shim_env();

    // Output shell script to stdout (for eval)
    println!("# Velo Rift Inception Mode - Enter the Dream");
    for (key, value) in &shim_env {
        println!("export {}=\"{}\"", key, value);
    }
    println!("export VRIFT_INCEPTION=1");
    // Phase 1.2: Export vDird socket + mmap paths for zero-RPC inception init
    if let Some(ref conn) = daemon_conn {
        if !conn.vdird_socket.is_empty() {
            println!("export VRIFT_VDIRD_SOCKET=\"{}\"", conn.vdird_socket);
        }
        if !conn.vdir_mmap_path.is_empty() {
            println!("export VRIFT_VDIR_MMAP=\"{}\"", conn.vdir_mmap_path);
        }
    }
    // Add both .vrift/bin (wrappers) and vrift binary dir to PATH
    if !vrift_bin_dir.is_empty() {
        println!(
            "export PATH=\"{}/.vrift/bin:{}:$PATH\"",
            project_root_str, vrift_bin_dir
        );
    } else {
        println!("export PATH=\"{}/.vrift/bin:$PATH\"", project_root_str);
    }
    #[cfg(target_os = "macos")]
    {
        println!(
            "export DYLD_INSERT_LIBRARIES=\"{}\"",
            inception_path.to_string_lossy()
        );
        println!("export DYLD_FORCE_FLAT_NAMESPACE=1");
    }
    #[cfg(target_os = "linux")]
    {
        println!("export LD_PRELOAD=\"{}\"", inception_path.to_string_lossy());
    }
    println!();
    println!("# Update prompt with totem");
    println!("export _VRIFT_OLD_PS1=\"$PS1\"");
    println!("export PS1=\"(vrift {}) $PS1\"", TOTEM_SPIN);
    println!();

    // Box output for inception complete
    let project_name = project_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    let stats_line = format!("VFS: {} files │ {}", file_count, cas_size);

    println!("echo ''");
    println!("echo '{}{}{}'", BOX_TL, BOX_H.repeat(35), BOX_TR);
    println!(
        "echo '{} {} INCEPTION                       {}'",
        BOX_V, TOTEM_SPIN, BOX_V
    );
    println!(
        "echo '{}                                    {}'",
        BOX_V, BOX_V
    );
    println!(
        "echo '{}    Project: {:<23} {}'",
        BOX_V,
        truncate_str(&project_name, 23),
        BOX_V
    );
    println!(
        "echo '{}    {:<31} {}'",
        BOX_V,
        truncate_str(&stats_line, 31),
        BOX_V
    );
    println!("echo '{}{}{}'", BOX_BL, BOX_H.repeat(35), BOX_BR);
    println!("echo ''");

    Ok(())
}

/// Generate shell script for exiting inception mode
pub fn cmd_wake() -> Result<()> {
    // RFC-0052: Deactivate persistent session
    // NOTE: raw env::var is intentional here. `cmd_wake` runs inside an
    // inception subshell where VRIFT_PROJECT_ROOT was set by `cmd_inception`
    // or `cmd_shell`. Config layer should NOT be used here — we need the
    // shell-level value that was exported, not a fresh TOML load.
    let project_root = env::var("VRIFT_PROJECT_ROOT").unwrap_or_else(|_| ".".to_string());
    let _ = crate::active::deactivate(Path::new(&project_root));

    // Theatrical wake animation (to stderr)
    show_wake_animation();

    // Output shell script to stdout (for eval)
    println!("# Velo Rift Wake - Exit the Dream");
    println!("unset VRIFT_PROJECT_ROOT");
    println!("unset VRIFT_INCEPTION");
    println!("unset VRIFT_MANIFEST");
    println!("unset VRIFT_VDIRD_SOCKET");
    println!("unset VRIFT_VDIR_MMAP");
    #[cfg(target_os = "macos")]
    {
        println!("unset DYLD_INSERT_LIBRARIES");
        println!("unset DYLD_FORCE_FLAT_NAMESPACE");
    }
    #[cfg(target_os = "linux")]
    {
        println!("unset LD_PRELOAD");
    }
    println!();
    println!("# Restore original PATH (remove .vrift/bin)");
    println!("export PATH=$(echo \"$PATH\" | sed 's|[^:]*/.vrift/bin:||g')");
    println!();
    println!("# Restore original prompt");
    println!("if [ -n \"$_VRIFT_OLD_PS1\" ]; then");
    println!("  export PS1=\"$_VRIFT_OLD_PS1\"");
    println!("  unset _VRIFT_OLD_PS1");
    println!("fi");
    println!();

    // Box output for wake complete
    println!("echo ''");
    println!("echo '{}{}{}'", BOX_TL, BOX_H.repeat(35), BOX_TR);
    println!(
        "echo '{} {} WAKE                            {}'",
        BOX_V, BELL, BOX_V
    );
    println!("echo '{}{}{}'", BOX_BL, BOX_H.repeat(35), BOX_BR);
    println!("echo ''");

    Ok(())
}

/// Generate shell hook for automatic inception/wake on cd
pub fn cmd_hook(shell: &str) -> Result<()> {
    match shell {
        "bash" => print_bash_hook(),
        "zsh" => print_zsh_hook(),
        "fish" => print_fish_hook(),
        _ => {
            eprintln!(
                "{} Unsupported shell: {}. Supported: bash, zsh, fish",
                style("Error:").red().bold(),
                shell
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

// ============================================================================
// Wrapper Generation (P1: PATH-based command shadowing)
// ============================================================================

/// Commands that need wrappers for SIP bypass on macOS
const WRAPPER_COMMANDS: &[&str] = &[
    "chmod", "chown", "rm", "cp", "mv", "touch", "mkdir", "rmdir",
];

/// Ensure .vrift/bin/ contains wrapper scripts for SIP-protected commands
fn ensure_wrappers(vrift_dir: &Path) -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let bin_dir = vrift_dir.join("bin");

    // Create bin directory if it doesn't exist
    if !bin_dir.exists() {
        fs::create_dir_all(&bin_dir)?;
    }

    // Generate wrappers for system commands
    for cmd in WRAPPER_COMMANDS {
        let wrapper_path = bin_dir.join(cmd);

        // Skip if wrapper already exists
        if wrapper_path.exists() {
            continue;
        }

        // Generate wrapper script
        let wrapper_content = generate_wrapper_script(cmd);
        fs::write(&wrapper_path, wrapper_content)?;

        // Make executable
        let mut perms = fs::metadata(&wrapper_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper_path, perms)?;
    }

    // Generate vrift wrapper (strips DYLD vars before calling real vrift)
    let vrift_wrapper_path = bin_dir.join("vrift");
    if !vrift_wrapper_path.exists() {
        if let Ok(real_vrift) = std::env::current_exe() {
            let vrift_wrapper = format!(
                r#"#!/bin/bash
# Velo Rift CLI wrapper - runs vrift without DYLD interference
# Auto-generated

unset DYLD_INSERT_LIBRARIES
unset DYLD_FORCE_FLAT_NAMESPACE
exec "{}" "$@"
"#,
                real_vrift.to_string_lossy()
            );
            fs::write(&vrift_wrapper_path, vrift_wrapper)?;
            let mut perms = fs::metadata(&vrift_wrapper_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&vrift_wrapper_path, perms)?;
        }
    }

    Ok(())
}

/// Generate a wrapper script for a command
///
/// These wrappers exist for SIP bypass on macOS - they allow the shim
/// to intercept syscalls from system binaries. The wrapper itself just
/// passes through to the real command; actual VFS logic is in the shim.
fn generate_wrapper_script(cmd: &str) -> String {
    // Find the real binary path (varies by platform)
    let real_path = match cmd {
        // macOS paths
        "chmod" | "rm" | "cp" | "mv" | "mkdir" | "rmdir" => format!("/bin/{}", cmd),
        "chown" => "/usr/sbin/chown".to_string(),
        "touch" => "/usr/bin/touch".to_string(),
        _ => format!("/usr/bin/{}", cmd),
    };

    format!(
        r#"#!/bin/bash
# Velo Rift SIP Bypass Wrapper for {cmd}
# Auto-generated - passes through to real command
# VFS interception happens at the shim/syscall level

exec {real_path} "$@"
"#,
        cmd = cmd,
        real_path = real_path
    )
}

// ============================================================================
// Private helpers
// ============================================================================

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        format!("{:<width$}", s, width = max_len)
    } else {
        let truncated: String = s.chars().take(max_len - 2).collect();
        format!("{}..", truncated)
    }
}

fn get_vfs_stats(vrift_dir: &Path) -> (String, String) {
    // Try to get file count from manifest
    let manifest_path = vrift_dir.join("manifest.lmdb");
    let file_count = if manifest_path.exists() {
        // Estimate from file size (rough approximation)
        if let Ok(meta) = std::fs::metadata(&manifest_path) {
            let size = meta.len();
            // Rough estimate: ~200 bytes per entry
            let estimated = size / 200;
            if estimated > 0 {
                format!("{}", estimated)
            } else {
                "?".to_string()
            }
        } else {
            "?".to_string()
        }
    } else {
        "0".to_string()
    };

    // Try to get CAS size
    let cas_size = "cached".to_string(); // Simplified for now

    (file_count, cas_size)
}

fn find_inception_library(project_root: &Path) -> Result<std::path::PathBuf> {
    let inception_name = if cfg!(target_os = "macos") {
        "libvrift_inception_layer.dylib"
    } else {
        "libvrift_inception_layer.so"
    };

    // Check local .vrift directory first
    let local_inception = project_root.join(".vrift").join(inception_name);
    if local_inception.exists() {
        return Ok(local_inception);
    }

    // Check relative to executable
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Same directory as vrift binary
            let sibling = exe_dir.join(inception_name);
            if sibling.exists() {
                return Ok(sibling);
            }

            // ../lib/ relative to bin/
            let lib_dir = exe_dir.parent().map(|p| p.join("lib").join(inception_name));
            if let Some(lib_path) = lib_dir {
                if lib_path.exists() {
                    return Ok(lib_path);
                }
            }
        }
    }

    // Check cargo target directory (development mode)
    let target_debug = Path::new("target/debug").join(inception_name);
    if target_debug.exists() {
        return normalize_for_ipc(target_debug).context("resolve target path");
    }

    let target_release = Path::new("target/release").join(inception_name);
    if target_release.exists() {
        return normalize_for_ipc(target_release).context("resolve target path");
    }

    anyhow::bail!(
        "Could not find {}. Please run 'cargo build -p vrift-inception-layer' first.",
        inception_name
    )
}

fn show_inception_animation() {
    let steps = [
        "Synchronizing blake3 synapses...",
        "Mapping LMDB memory layers...",
        "Injecting VFS into shell context...",
        "Stabilizing dream architecture...",
    ];

    eprintln!();
    eprintln!(
        "{}",
        style("▼ INITIALIZING VIRTUAL RIFT PROTOCOL").cyan().bold()
    );
    eprintln!();

    for step in steps {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.enable_steady_tick(Duration::from_millis(80));
        pb.set_message(step.to_string());

        // Simulate processing
        std::thread::sleep(Duration::from_millis(300));

        pb.finish_with_message(format!("{} {}", CHECK, style(step).green()));
    }

    eprintln!();
}

fn show_wake_animation() {
    eprintln!();
    eprintln!("{}", style("▲ EXITING VIRTUAL RIFT LAYER").cyan().bold());

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠏⠇⠧⠦⠴⠼⠸⠹⠙⠋")
            .template("{spinner:.yellow} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(Duration::from_millis(80));
    pb.set_message("Dissolving virtual projections...");

    std::thread::sleep(Duration::from_millis(400));

    pb.finish_with_message(format!("{} {}", CHECK, style("Dream collapsed").green()));
    eprintln!();
}

fn print_bash_hook() {
    println!(
        r#"# Velo Rift Auto-Inception Hook for Bash
# Add to ~/.bashrc: eval "$(vrift hook bash)"

_vrift_hook() {{
    local vrift_dir="$(pwd)/.vrift"
    
    if [[ -d "$vrift_dir" ]] && [[ -z "$VRIFT_INCEPTION" ]]; then
        # Entering a VFS project - auto inception
        eval "$(vrift inception 2>/dev/null)"
    elif [[ ! -d "$vrift_dir" ]] && [[ -n "$VRIFT_INCEPTION" ]]; then
        # Left the VFS project - auto wake
        eval "$(vrift wake 2>/dev/null)"
    fi
}}

# Hook into cd
cd() {{
    builtin cd "$@" && _vrift_hook
}}

# Also hook pushd/popd for completeness
pushd() {{
    builtin pushd "$@" && _vrift_hook
}}

popd() {{
    builtin popd "$@" && _vrift_hook
}}

# Run on shell init
_vrift_hook
"#
    );
}

fn print_zsh_hook() {
    println!(
        r#"# Velo Rift Auto-Inception Hook for Zsh
# Add to ~/.zshrc: eval "$(vrift hook zsh)"

_vrift_hook() {{
    local vrift_dir="$(pwd)/.vrift"
    
    if [[ -d "$vrift_dir" ]] && [[ -z "$VRIFT_INCEPTION" ]]; then
        eval "$(vrift inception 2>/dev/null)"
    elif [[ ! -d "$vrift_dir" ]] && [[ -n "$VRIFT_INCEPTION" ]]; then
        eval "$(vrift wake 2>/dev/null)"
    fi
}}

# Use chpwd hook (called after every directory change)
autoload -U add-zsh-hook
add-zsh-hook chpwd _vrift_hook

# Run on shell init
_vrift_hook
"#
    );
}

fn print_fish_hook() {
    println!(
        r#"# Velo Rift Auto-Inception Hook for Fish
# Add to ~/.config/fish/config.fish: vrift hook fish | source

function _vrift_hook --on-variable PWD
    set -l vrift_dir (pwd)/.vrift
    
    if test -d $vrift_dir; and not set -q VRIFT_INCEPTION
        vrift inception 2>/dev/null | source
    else if not test -d $vrift_dir; and set -q VRIFT_INCEPTION
        vrift wake 2>/dev/null | source
    end
end

# Run on shell init
_vrift_hook
"#
    );
}
