//! vdir_d - Per-project Virtual Directory Daemon
//!
//! Usage:
//!   vdir_d /path/to/project

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;
use vrift_vdird::{run_daemon, ProjectConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("vrift_vdird=debug".parse().unwrap()),
        )
        .init();

    // Parse args
    let args: Vec<String> = std::env::args().collect();
    let project_root = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        std::env::current_dir().context("Failed to get current directory")?
    };

    // Validate project root exists
    if !project_root.exists() {
        anyhow::bail!("Project root does not exist: {}", project_root.display());
    }

    let project_root = project_root
        .canonicalize()
        .context("Failed to canonicalize project root")?;

    info!(path = %project_root.display(), "Starting vdir_d for project");

    // Create config and run
    let config = ProjectConfig::from_project_root(project_root);
    run_daemon(config).await
}
