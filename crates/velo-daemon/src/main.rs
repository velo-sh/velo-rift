use anyhow::Result;
use clap::{Parser, Subcommand};

use tokio::signal;

#[derive(Parser)]
#[command(name = "velod")]
#[command(version, about = "Velo Rift Daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon (default)
    Start,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Start) {
        Commands::Start => start_daemon().await?,
    }

    Ok(())
}

async fn start_daemon() -> Result<()> {
    println!("velod: Starting daemon...");

    // In a real implementation, we would manage the socket file location properly
    // and handle cleanup. For MVP, we'll just print a message.
    // let socket_path = "/tmp/velod.sock";
    // ... socket binding logic ...

    println!("velod: Ready to accept connections (placeholder)");

    // Wait for shutdown signal
    match signal::ctrl_c().await {
        Ok(()) => {
            println!("velod: Shutdown signal received");
        }
        Err(err) => {
            eprintln!("velod: Unable to listen for shutdown signal: {}", err);
        }
    }

    println!("velod: Shutting down");
    Ok(())
}
