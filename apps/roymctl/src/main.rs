#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Command-line control interface for operations on, and through, a Syneroym
//! substrate.

use std::{path::PathBuf, process};

use clap::Parser;
use commands::Commands;
use rustls::crypto::ring;

mod commands;

/// Default API endpoint for the Community Registry
const DEFAULT_API_URL: &str = "http://localhost:7961";

#[derive(Parser)]
#[command(name = "roymctl")]
#[command(about = "Syneroym Control CLI (API Client)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The base URL of the Syneroym Community Registry
    #[arg(global = true, long, default_value = DEFAULT_API_URL)]
    api_url: String,

    /// The DID of the target substrate to control
    #[arg(global = true, long)]
    substrate: Option<String>,

    /// Local directory for configuration and offline identities (defaults to
    /// current dir)
    #[arg(global = true, long, default_value = ".")]
    dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if ring::default_provider().install_default().is_err() {
        eprintln!("Failed to install rustls default crypto provider");
        process::exit(1);
    }

    let cli = Cli::parse();

    commands::run(cli.command, cli.api_url, cli.substrate, cli.dir).await?;

    Ok(())
}
