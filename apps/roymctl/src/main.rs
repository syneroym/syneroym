//! Command-line control interface for operations on, and through, a Syneroym substrate.

use clap::Parser;
use std::path::PathBuf;

mod commands;

/// Default API endpoint for the Community Registry
const DEFAULT_API_URL: &str = "http://localhost:7961";

#[derive(Parser)]
#[command(name = "roymctl")]
#[command(about = "Syneroym Control CLI (API Client)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: commands::Commands,

    /// The base URL of the Syneroym Community Registry
    #[arg(global = true, long, default_value = DEFAULT_API_URL)]
    api_url: String,

    /// The DID of the target substrate to control
    #[arg(global = true, long)]
    substrate: Option<String>,

    /// Local directory for configuration and offline identities (defaults to current dir)
    #[arg(global = true, long, default_value = ".")]
    dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls default crypto provider");

    let cli = Cli::parse();

    commands::run(cli.command, cli.api_url, cli.substrate, cli.dir).await?;

    Ok(())
}
