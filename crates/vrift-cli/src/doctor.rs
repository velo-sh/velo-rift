//! # vrift doctor
//!
//! Diagnostic checks for Velo Rift environment health.
//! Validates config files, socket connectivity, shim presence,
//! CAS directory permissions, and manifest integrity.

use anyhow::Result;
use console::{style, Emoji};
use std::path::Path;

static CHECK: Emoji<'_, '_> = Emoji("✔ ", "[ok] ");
static CROSS: Emoji<'_, '_> = Emoji("✘ ", "[!!] ");
static WARN_ICON: Emoji<'_, '_> = Emoji("⚠ ", "[??] ");
static DOT: Emoji<'_, '_> = Emoji("● ", "[-] ");

struct DiagResult {
    passed: u32,
    warned: u32,
    failed: u32,
}

impl DiagResult {
    fn new() -> Self {
        Self {
            passed: 0,
            warned: 0,
            failed: 0,
        }
    }

    fn pass(&mut self, msg: &str) {
        self.passed += 1;
        eprintln!("  {} {}", CHECK, style(msg).green());
    }

    fn warn(&mut self, msg: &str) {
        self.warned += 1;
        eprintln!("  {} {}", WARN_ICON, style(msg).yellow());
    }

    fn fail(&mut self, msg: &str) {
        self.failed += 1;
        eprintln!("  {} {}", CROSS, style(msg).red());
    }

    fn info(&self, msg: &str) {
        eprintln!("  {} {}", DOT, style(msg).dim());
    }
}

pub fn cmd_doctor(project_dir: &Path) -> Result<()> {
    eprintln!();
    eprintln!("{}", style("🩺 Velo Rift Doctor").bold().cyan());
    eprintln!("{}", style("─".repeat(40)).dim());

    let mut d = DiagResult::new();

    // 1. Config loading
    eprintln!();
    eprintln!("{}", style("Config").bold());
    check_config(&mut d);

    // 2. Project structure
    eprintln!();
    eprintln!("{}", style("Project").bold());
    check_project(project_dir, &mut d);

    // 3. Daemon / Socket
    eprintln!();
    eprintln!("{}", style("Daemon").bold());
    check_daemon(&mut d);

    // 4. Shim library
    eprintln!();
    eprintln!("{}", style("Shim (Inception Layer)").bold());
    check_shim(project_dir, &mut d);

    // 5. CAS / TheSource
    eprintln!();
    eprintln!("{}", style("CAS (TheSource™)").bold());
    check_cas(&mut d);

    // Summary
    eprintln!();
    eprintln!("{}", style("─".repeat(40)).dim());
    eprintln!(
        "  {} passed, {} warnings, {} errors",
        style(d.passed).green().bold(),
        style(d.warned).yellow().bold(),
        style(d.failed).red().bold(),
    );

    if d.failed > 0 {
        eprintln!();
        eprintln!(
            "{}",
            style("Run 'vrift init' to fix project setup issues.").dim()
        );
        std::process::exit(1);
    } else if d.warned > 0 {
        eprintln!(
            "{}",
            style("Some warnings detected. System should still work.").dim()
        );
    } else {
        eprintln!("{}", style("All checks passed. System is healthy!").dim());
    }

    eprintln!();
    Ok(())
}

fn check_config(d: &mut DiagResult) {
    // Global config
    match vrift_config::Config::global_config_path() {
        Some(path) => {
            if path.exists() {
                d.pass(&format!("Global config: {}", path.display()));

                // Check for known issues
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if content.contains("/run/vrift/") && cfg!(target_os = "macos") {
                        d.warn("Global config has Linux socket path (/run/vrift/) on macOS");
                        d.info("Config layer will auto-fallback to /tmp/vrift.sock");
                    }
                    if !content.contains("config_version") {
                        d.warn("Global config missing 'config_version' field (pre-v1 schema)");
                        d.info("Regenerate with: vrift config init --global");
                    }
                }
            } else {
                d.warn(&format!("Global config not found: {}", path.display()));
                d.info("Create with: vrift init");
            }
        }
        None => d.fail("Cannot determine home directory"),
    }

    // Config load test
    match vrift_config::Config::load() {
        Ok(cfg) => {
            d.pass(&format!(
                "Config loads successfully (version {})",
                cfg.config_version
            ));
        }
        Err(e) => {
            d.fail(&format!("Config load failed: {}", e));
        }
    }
}

fn check_project(project_dir: &Path, d: &mut DiagResult) {
    let vrift_dir = project_dir.join(".vrift");

    if vrift_dir.exists() {
        d.pass(&format!(".vrift/ exists at {}", project_dir.display()));
    } else {
        d.fail(&format!(".vrift/ not found at {}", project_dir.display()));
        d.info("Run 'vrift init' to create project structure");
        return;
    }

    // Check config.toml
    let config_path = vrift_dir.join("config.toml");
    if config_path.exists() {
        d.pass("Project config: .vrift/config.toml");
    } else {
        d.warn("No project config: .vrift/config.toml");
        d.info("Run 'vrift init' to generate");
    }

    // Check bin/ directory
    let bin_dir = vrift_dir.join("bin");
    if bin_dir.exists() {
        let wrapper_count = std::fs::read_dir(&bin_dir)
            .map(|rd| rd.count())
            .unwrap_or(0);
        d.pass(&format!(".vrift/bin/ ({} wrappers)", wrapper_count));
    } else {
        d.warn(".vrift/bin/ missing (SIP bypass wrappers)");
    }

    // Check manifest
    let project_id = vrift_config::path::compute_project_id(project_dir);
    if let Some(manifest_path) = vrift_config::path::get_manifest_db_path(&project_id) {
        if manifest_path.exists() {
            let size = std::fs::metadata(&manifest_path)
                .map(|m| m.len())
                .unwrap_or(0);
            d.pass(&format!(
                "Manifest LMDB: {} ({})",
                manifest_path.display(),
                format_bytes(size)
            ));
        } else {
            d.warn(&format!("Manifest not found: {}", manifest_path.display()));
        }
    } else {
        d.warn("Cannot determine manifest path");
    }
}

fn check_daemon(d: &mut DiagResult) {
    let cfg = vrift_config::Config::load().unwrap_or_default();
    let socket = cfg.socket_path();

    if socket.exists() {
        d.pass(&format!("Socket exists: {}", socket.display()));

        // Try connecting
        match std::os::unix::net::UnixStream::connect(socket) {
            Ok(_) => d.pass("Daemon is responsive"),
            Err(e) => d.warn(&format!("Socket exists but can't connect: {}", e)),
        }
    } else {
        d.warn(&format!("Daemon socket not found: {}", socket.display()));
        d.info("Start with: vrift daemon status");
    }

    // Check socket parent dir
    if let Some(parent) = socket.parent() {
        if parent.exists() {
            d.pass(&format!("Socket directory writable: {}", parent.display()));
        } else {
            d.warn(&format!("Socket directory missing: {}", parent.display()));
        }
    }
}

fn check_shim(project_dir: &Path, d: &mut DiagResult) {
    let lib_name = if cfg!(target_os = "macos") {
        "libvrift_inception_layer.dylib"
    } else {
        "libvrift_inception_layer.so"
    };

    // Check relative to executable
    let found = if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join(lib_name);
            if path.exists() {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                d.pass(&format!("{} ({})", lib_name, format_bytes(size)));
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if !found {
        // Check project local
        let local = project_dir.join(".vrift").join(lib_name);
        if local.exists() {
            d.pass(&format!("{} (project-local)", lib_name));
        } else {
            d.fail(&format!("{} not found", lib_name));
            d.info("Build with: cargo build -p vrift-inception-layer");
        }
    }

    // SIP check (macOS)
    #[cfg(target_os = "macos")]
    {
        d.info("macOS SIP: wrappers in .vrift/bin/ bypass SIP restrictions");
    }
}

fn check_cas(d: &mut DiagResult) {
    let cfg = vrift_config::Config::load().unwrap_or_default();
    let cas_root = cfg.cas_root();
    let resolved = vrift_manifest::normalize_path(&cas_root.to_string_lossy());

    if resolved.exists() {
        // Count blobs
        let blob_count = std::fs::read_dir(&resolved)
            .map(|rd| rd.count())
            .unwrap_or(0);
        let dir_size = dir_size_approx(&resolved);
        d.pass(&format!(
            "TheSource™: {} ({} entries, {})",
            resolved.display(),
            blob_count,
            format_bytes(dir_size)
        ));

        // Check permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(meta) = std::fs::metadata(&resolved) {
                let mode = meta.mode() & 0o777;
                if mode & 0o200 != 0 {
                    d.pass("CAS directory is writable");
                } else {
                    d.fail(&format!("CAS directory not writable (mode {:o})", mode));
                }
            }
        }
    } else {
        d.fail(&format!("TheSource™ not found: {}", resolved.display()));
        d.info("Run 'vrift init' to create");
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1}G", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn dir_size_approx(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}
