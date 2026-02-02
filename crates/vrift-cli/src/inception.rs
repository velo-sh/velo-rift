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

// Emojis for theatrical effect
static TOTEM: Emoji<'_, '_> = Emoji("🌀 ", "* ");
static CHECK: Emoji<'_, '_> = Emoji("✔ ", "[ok] ");
static WAKE: Emoji<'_, '_> = Emoji("💫 ", "~ ");
static WARN: Emoji<'_, '_> = Emoji("⚠️  ", "! ");

/// Generate shell script for `eval "$(vrift inception)"`
pub fn cmd_inception(project_dir: &Path) -> Result<()> {
    let vrift_dir = project_dir.join(".vrift");

    // Check if this is a valid VFS project
    if !vrift_dir.exists() {
        eprintln!(
            "{} {}",
            style("Error:").red().bold(),
            style("No .vrift directory found. Run 'vrift init' first.").red()
        );
        std::process::exit(1);
    }

    // Check if already in inception
    if env::var("VRIFT_INCEPTION").is_ok() {
        eprintln!(
            "{} {}",
            WARN,
            style("Already in inception mode. Use 'vrift wake' to exit first.").yellow()
        );
        std::process::exit(0);
    }

    let project_root = project_dir
        .canonicalize()
        .context("Failed to resolve project path")?;
    let project_root_str = project_root.to_string_lossy();

    // Find the shim library
    let shim_path = find_shim_library(&project_root)?;

    // Theatrical progress (to stderr so it doesn't interfere with eval)
    show_inception_animation();

    // Output shell script to stdout (for eval)
    println!("# Velo Rift Inception Mode - Enter the Dream");
    println!("export VRIFT_PROJECT_ROOT=\"{}\"", project_root_str);
    println!("export VRIFT_INCEPTION=1");
    println!(
        "export VRIFT_MANIFEST=\"{}/.vrift/manifest.lmdb\"",
        project_root_str
    );
    println!("export PATH=\"{}/.vrift/bin:$PATH\"", project_root_str);
    println!(
        "export DYLD_INSERT_LIBRARIES=\"{}\"",
        shim_path.to_string_lossy()
    );
    println!("export DYLD_FORCE_FLAT_NAMESPACE=1");
    println!();
    println!("# Update prompt with totem");
    println!("export _VRIFT_OLD_PS1=\"$PS1\"");
    println!("export PS1=\"(vrift 🌀) $PS1\"");
    println!();
    println!("echo ''");
    println!(
        "echo '{} {}'",
        TOTEM,
        style("INCEPTION COMPLETE").green().bold()
    );
    println!(
        "echo '   {} {}'",
        style("Status:").dim(),
        "Reality distorted. Happy hacking."
    );
    println!("echo '   {} {}'", style("Project:").dim(), project_root_str);
    println!("echo ''");

    Ok(())
}

/// Generate shell script for exiting inception mode
pub fn cmd_wake() -> Result<()> {
    // Check if in inception
    if env::var("VRIFT_INCEPTION").is_err() {
        eprintln!(
            "{} {}",
            WARN,
            style("Not in inception mode. Nothing to wake from.").yellow()
        );
        std::process::exit(0);
    }

    // Theatrical wake animation (to stderr)
    show_wake_animation();

    // Output shell script to stdout (for eval)
    println!("# Velo Rift Wake - Exit the Dream");
    println!("unset VRIFT_PROJECT_ROOT");
    println!("unset VRIFT_INCEPTION");
    println!("unset VRIFT_MANIFEST");
    println!("unset DYLD_INSERT_LIBRARIES");
    println!("unset DYLD_FORCE_FLAT_NAMESPACE");
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
    println!(
        "echo '{} {}'",
        WAKE,
        style("Wake: Back to reality.").cyan().bold()
    );

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
// Private helpers
// ============================================================================

fn find_shim_library(project_root: &Path) -> Result<std::path::PathBuf> {
    // Check local .vrift directory first
    let local_shim = project_root.join(".vrift/libvrift_shim.dylib");
    if local_shim.exists() {
        return Ok(local_shim);
    }

    // Check relative to executable
    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Same directory as vrift binary
            let sibling = exe_dir.join("libvrift_shim.dylib");
            if sibling.exists() {
                return Ok(sibling);
            }

            // ../lib/ relative to bin/
            let lib_dir = exe_dir.parent().map(|p| p.join("lib/libvrift_shim.dylib"));
            if let Some(lib_path) = lib_dir {
                if lib_path.exists() {
                    return Ok(lib_path);
                }
            }
        }
    }

    // Check cargo target directory (development mode)
    let target_debug = Path::new("target/debug/libvrift_shim.dylib");
    if target_debug.exists() {
        return target_debug.canonicalize().context("resolve target path");
    }

    let target_release = Path::new("target/release/libvrift_shim.dylib");
    if target_release.exists() {
        return target_release.canonicalize().context("resolve target path");
    }

    anyhow::bail!(
        "Could not find libvrift_shim.dylib. Please run 'cargo build -p vrift-shim' first."
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
